use core::time;
use std::{
    cell::{Cell, RefCell},
    env,
    fs::File,
    io::Cursor,
    thread,
};

use anyhow::Result;
use epub_builder::{EpubBuilder, EpubContent, EpubVersion, ReferenceType, TocElement, ZipCommand};
use html5ever::{
    tendril::{ByteTendril, ReadExt},
    tokenizer::{
        BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
    },
};
use ureq::{Agent, BodyReader};

trait SinkType: Default {}
impl SinkType for LinksSink {}
impl SinkType for ChapterSink {}

#[derive(Default)]
struct ChapterSink {
    found_name: Cell<bool>,
    found_content: Cell<bool>,
    title: RefCell<String>,
    text: RefCell<String>,
}

impl TokenSink for ChapterSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<()> {
        match token {
            Token::TagToken(tag) => match tag.kind {
                TagKind::StartTag => {
                    for attr in &tag.attrs {
                        match (attr.name.local.as_ref(), attr.value.as_ref()) {
                            ("class", "name") => self.found_name.set(true),
                            ("class", "content") => self.found_content.set(true),
                            (_, _) => {}
                        }
                    }
                }
                TagKind::EndTag => match (self.found_name.get(), self.found_content.get()) {
                    (true, false) => self.found_name.set(false),
                    (false, true) => self.found_content.set(false),
                    (_, _) => {}
                },
            },
            Token::CharacterTokens(text) => {
                match (self.found_name.get(), self.found_content.get()) {
                    (true, false) => self.title.borrow_mut().push_str(text.as_ref()),
                    (false, true) => {
                        if text.is_empty() {
                            return TokenSinkResult::Continue;
                        }
                        let trimmed = text.replace('\n', "<br />").replace("\u{2003}", "");
                        self.text.borrow_mut().push_str(&trimmed.to_string());
                    }
                    (_, _) => {}
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

#[derive(Default)]
struct LinksSink {
    links: RefCell<Vec<String>>,
    author: Cell<String>,
    title: Cell<String>,
    found_author_tag: Cell<bool>,
    found_author: Cell<bool>,
    found_title: Cell<bool>,
    found_links: Cell<bool>,
}

impl TokenSink for LinksSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<()> {
        match token {
            Token::TagToken(tag) => match tag.kind {
                TagKind::StartTag => match tag.name.as_ref() {
                    "span" => {
                        for attr in &tag.attrs {
                            match (attr.name.local.as_ref(), attr.value.as_ref()) {
                                ("class", "author") => self.found_author_tag.set(true),
                                ("class", "title") => self.found_title.set(true),
                                (_, _) => {}
                            }
                        }
                    }
                    "a" => match (self.found_author_tag.get(), self.found_links.get()) {
                        (true, false) => self.found_author.set(true),
                        (false, true) => {
                            for attr in &tag.attrs {
                                if attr.name.local.as_ref() == "href" {
                                    self.links
                                        .borrow_mut()
                                        .push(format!("https:{}", attr.value.as_ref()));
                                }
                            }
                        }
                        (_, _) => {}
                    },
                    "ul" => {
                        for attr in &tag.attrs {
                            if let ("id", "chapter-list") =
                                (attr.name.local.as_ref(), attr.value.as_ref())
                            {
                                self.found_links.set(true);
                            }
                        }
                    }
                    _ => {}
                },
                TagKind::EndTag => match (
                    self.found_author.get(),
                    self.found_title.get(),
                    self.found_links.get(),
                ) {
                    (true, false, false) => {
                        self.found_author.set(false);
                        self.found_author_tag.set(false);
                    }
                    (false, true, false) => self.found_title.set(false),
                    (false, false, true) => {
                        if tag.name.as_ref() == "ul" {
                            self.found_links.set(false);
                        }
                    }
                    (_, _, _) => {}
                },
            },
            Token::CharacterTokens(text) => {
                match (self.found_author.get(), self.found_title.get()) {
                    (true, false) => {
                        self.author.set(text.to_string());
                    }
                    (false, true) => {
                        self.title.set(text.to_string());
                    }
                    (_, _) => {}
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn main() {
    simple_logger::init_with_level(log::Level::Info).expect("simple logger init failed");

    let args: Vec<String> = env::args().collect();
    let agent = Agent::new_with_defaults();
    let mut book = EpubBuilder::new(ZipCommand::new().expect("new zip command failed"))
        .expect("new epub builder failed");

    book.epub_version(EpubVersion::V30);

    let info: LinksSink = process::<LinksSink>(&agent, &args[1]).expect("process_info failed");

    book.add_author(info.author.into_inner());
    let title = info.title.into_inner();
    book.set_title(title.clone());
    let links = info.links.into_inner();

    for (i, item) in links.iter().enumerate() {
        log::info!("{item}");

        let content: ChapterSink =
            process::<ChapterSink>(&agent, item).expect("process_chapter failed");
        let title = content.title.into_inner();

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
                    content.text.into_inner()
                )),
            )
            .title(title.clone())
            .reftype(ReferenceType::Text)
            .child(TocElement::new(format!("{i}.xhtml#1"), title)),
        )
        .expect("create chapter failed");
    }

    book.inline_toc();

    let mut output_file = File::create(format!("{title}.epub")).expect("create epub file failed");
    book.generate(&mut output_file)
        .expect("epub generate failed");
}

fn process<T: SinkType + TokenSink>(agent: &Agent, path: &str) -> Result<T> {
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
            Err(ureq::Error::StatusCode(429)) => {
                log::info!("received 429, retrying in {delay:?}");
                thread::sleep(delay);
                delay *= 2;
                retries -= 1;
            }
            Err(e) => return Err(anyhow::anyhow!(e)),
        }
    }

    Err(anyhow::anyhow!("max retries exceeded"))
}
