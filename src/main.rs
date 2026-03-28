use std::{env, fs::File, io::Cursor, str::FromStr, thread};

use anyhow::{Context, Result};
use core::time;
use epub_builder::{EpubBuilder, EpubContent, EpubVersion, ReferenceType, TocElement, ZipCommand};
use html5ever::{
    tendril::{ByteTendril, ReadExt},
    tokenizer::{BufferQueue, TokenSink, Tokenizer, TokenizerOpts},
};
use http::Uri;
use indicatif::{ProgressBar, ProgressStyle};
use ureq::{Agent, BodyReader, unversioned::multipart::Form};

mod download;

fn main() {
    let args: Vec<String> = env::args().collect();
    let agent = Agent::new_with_defaults();

    if args.len() < 2 {
        print_usage(&args[0]);
        return;
    }

    let command = &args[1];

    match command.as_str() {
        "fetch" => {
            let mut opts = getopts::Options::new();
            opts.optflag("h", "help", "print this help menu");

            let matches = match opts.parse(&args[2..]) {
                Ok(m) => m,
                Err(f) => {
                    eprintln!("{f}");
                    std::process::exit(1);
                }
            };

            if matches.opt_present("h") {
                let brief = format!("Usage: {} fetch [options] <URL>...", args[0]);
                print!("{}", opts.usage(&brief));
                return;
            }

            if matches.free.is_empty() {
                eprintln!("Error: Missing URL for fetch command");
                std::process::exit(1);
            }

            for u in &matches.free {
                let url = Uri::from_str(u).expect("Parse Uri failed");
                if let Err(e) = process_book(&url, &agent) {
                    eprintln!("Failed to process {url}: {e}");
                }
            }
        }
        "send" => {
            let mut opts = getopts::Options::new();
            opts.optflag("h", "help", "print this help menu");
            opts.optopt("k", "key", "4-character key for send.djazz.se", "KEY");
            opts.optflag("", "no-kepubify", "disable Kepubify conversion (Kobo only)");
            opts.optflag(
                "",
                "no-kindlegen",
                "disable KindleGen conversion (Kindle only)",
            );

            let matches = match opts.parse(&args[2..]) {
                Ok(m) => m,
                Err(f) => {
                    eprintln!("{f}");
                    std::process::exit(1);
                }
            };

            if matches.opt_present("h") {
                let brief = format!("Usage: {} send [options] <FILE.epub>...", args[0]);
                print!("{}", opts.usage(&brief));
                return;
            }

            let Some(key) = matches.opt_str("k") else {
                eprintln!("Error: -k/--key is required for the send command");
                std::process::exit(1);
            };

            if matches.free.is_empty() {
                eprintln!("Error: Missing file path for send command");
                std::process::exit(1);
            }

            let kepubify = !matches.opt_present("no-kepubify");
            let kindlegen = !matches.opt_present("no-kindlegen");

            for file in &matches.free {
                if let Err(e) = send_to_djazz(&agent, file, &key, kepubify, kindlegen) {
                    eprintln!("Failed to send {file}: {e}");
                }
            }
        }
        _ => {
            eprintln!("Unknown command: {command}");
            print_usage(&args[0]);
            std::process::exit(1);
        }
    }
}

fn print_usage(program: &str) {
    println!("Usage: {program} <command> [args]...");
    println!();
    println!("Commands:");
    println!("  fetch <URL>...    Fetch a book from a supported URL and build an epub");
    println!(
        "  send [options] <FILE.epub>...  Send an existing epub to a Kobo/Kindle using send.djazz.se"
    );
    println!();
    println!("Run `{program} <command> --help` for more information on a command.");
}

fn send_to_djazz(
    agent: &Agent,
    epub_path: &str,
    key: &str,
    kepubify: bool,
    kindlegen: bool,
) -> Result<()> {
    println!("Sending {epub_path} to Djazz (key: {key})...");

    let mut form = Form::new().text("key", key);

    if kepubify {
        form = form.text("kepubify", "on"); // Kobo conversion
    }

    if kindlegen {
        form = form.text("kindlegen", "on"); // Kindle conversion
    }

    let form = form
        .file("file", epub_path)
        .context("Failed to attach file to multipart form")?;

    let mut response = agent.post("https://send.djazz.se/upload").send(form)?;
    let status = response.status();
    let body = response.body_mut().read_to_string()?;

    if status == 200 {
        println!("Successfully sent to Djazz: {body}");
    } else {
        eprintln!("Failed to send to Djazz (Status {status}): {body}");
    }

    Ok(())
}

fn process_book(uri: &Uri, agent: &Agent) -> Result<()> {
    match uri.host() {
        Some("czbooks.net") => {
            process_book_with_provider::<download::czbooksnet::CzBooksProvider>(uri, agent)
        }
        _ => anyhow::bail!("Unsupported domain: {uri}"),
    }
}

fn process_book_with_provider<P: download::Provider>(uri: &Uri, agent: &Agent) -> Result<()> {
    let mut book = EpubBuilder::new(ZipCommand::new()?)?;

    book.epub_version(EpubVersion::V33);

    let info: download::BookInfo = process::<P::Link>(agent, uri)?.into();

    book.add_author(info.author);
    let title = info.title;
    book.set_title(title.clone());
    let links = info.links;

    let bar = ProgressBar::new(links.len() as u64);
    bar.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
            )?
            .progress_chars("#>-"),
    );
    bar.set_message(format!("Processing {title}"));

    for (i, item) in links.iter().enumerate() {
        let content_result = process::<P::Chapter>(agent, item);

        if let Err(e) = content_result {
            bar.abandon_with_message(format!("Failed to process chapter: {e}"));
            return Err(e);
        }

        let content_sink = content_result?;
        let content: download::ChapterInfo = content_sink.into();
        let chapter_title = content.title;

        book.add_content(
            EpubContent::new(
                format!("{i}.xhtml"),
                Cursor::new(format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
        <html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
        <body>
        {}
        </body>
        </html>"#,
                    content.text
                )),
            )
            .title(chapter_title.clone())
            .reftype(ReferenceType::Text)
            .child(TocElement::new(format!("{i}.xhtml#1"), chapter_title)),
        )?;
        bar.inc(1);
    }

    bar.finish_with_message("Done");
    book.inline_toc();

    let mut output_file = File::create(format!("{title}.epub"))?;
    book.generate(&mut output_file)?;

    Ok(())
}

fn process<T: Default + TokenSink<Handle = ()>>(agent: &Agent, path: &Uri) -> Result<T> {
    let mut resp = fetch_with_backoff(agent, path)?;
    let mut chunk = ByteTendril::new();

    resp.read_to_tendril(&mut chunk)?;

    let input = BufferQueue::default();
    input.push_back(
        chunk
            .try_reinterpret()
            .map_err(|e| anyhow::Error::msg(format!("try_reinterpret failed on:{e:?}")))?,
    );

    let sinker = T::default();
    let tok = Tokenizer::new(sinker, TokenizerOpts::default());
    let _ = tok.feed(&input);
    tok.end();

    Ok(tok.sink)
}

fn fetch_with_backoff(agent: &Agent, path: &Uri) -> Result<BodyReader<'static>> {
    let mut retries = 3;
    let mut delay = time::Duration::from_millis(3000);

    while retries > 0 {
        match agent.get(path).call() {
            Ok(resp) => {
                thread::sleep(time::Duration::from_millis(900));
                return Ok(resp.into_body().into_reader());
            }
            Err(ureq::Error::StatusCode(code)) if (400..=499).contains(&code) => {
                thread::sleep(delay);
                delay *= 2;
                retries -= 1;
            }
            Err(e) => return Err(anyhow::anyhow!(e)),
        }
    }

    Err(anyhow::anyhow!("max retries exceeded"))
}
