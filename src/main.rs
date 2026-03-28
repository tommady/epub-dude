use std::{env, fs::File, io::Cursor, thread};

use anyhow::Result;
use core::time;
use epub_builder::{EpubBuilder, EpubContent, EpubVersion, ReferenceType, TocElement, ZipCommand};
use html5ever::{
    tendril::{ByteTendril, ReadExt},
    tokenizer::{BufferQueue, TokenSink, Tokenizer, TokenizerOpts},
};
use indicatif::{ProgressBar, ProgressStyle};
use ureq::{Agent, BodyReader};

mod download;

fn main() {
    let args: Vec<String> = env::args().collect();
    let agent = Agent::new_with_defaults();

    for url in args.iter().skip(1) {
        if let Err(e) = process_book(url, &agent) {
            eprintln!("Failed to process {url}: {e}");
        }
    }
}

fn process_book(url: &str, agent: &Agent) -> Result<()> {
    if url.contains("czbooks.net") {
        process_book_with_provider::<download::czbooksnet::CzBooksProvider>(url, agent)
    } else {
        anyhow::bail!("Unsupported domain: {url}");
    }
}

fn process_book_with_provider<P: download::Provider>(url: &str, agent: &Agent) -> Result<()> {
    let mut book = EpubBuilder::new(ZipCommand::new()?)?;

    book.epub_version(EpubVersion::V33);

    let info: download::BookInfo = process::<P::Link>(agent, url)?.into();

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

fn process<T: Default + TokenSink<Handle = ()>>(agent: &Agent, path: &str) -> Result<T> {
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

fn fetch_with_backoff(agent: &Agent, path: &str) -> Result<BodyReader<'static>> {
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
