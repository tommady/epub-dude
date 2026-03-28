use super::{BookInfo, ChapterInfo, Provider};
use html5ever::tokenizer::{TagKind, Token, TokenSink, TokenSinkResult};
use std::cell::{Cell, RefCell};

pub struct CzBooksProvider;

impl Provider for CzBooksProvider {
    type Link = LinksSink;
    type Chapter = ChapterSink;
}

#[derive(Default)]
pub struct ChapterSink {
    found_name: Cell<bool>,
    found_content: Cell<bool>,
    title: RefCell<String>,
    text: RefCell<String>,
}

impl From<ChapterSink> for ChapterInfo {
    fn from(val: ChapterSink) -> Self {
        ChapterInfo {
            title: val.title.into_inner(),
            text: val.text.into_inner(),
        }
    }
}

#[derive(Default)]
pub struct LinksSink {
    links: RefCell<Vec<String>>,
    author: Cell<String>,
    title: Cell<String>,
    found_author_tag: Cell<bool>,
    found_author: Cell<bool>,
    found_title: Cell<bool>,
    found_links: Cell<bool>,
}

impl From<LinksSink> for BookInfo {
    fn from(val: LinksSink) -> Self {
        BookInfo {
            author: val.author.into_inner(),
            title: val.title.into_inner(),
            links: val.links.into_inner(),
        }
    }
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
                        self.text.borrow_mut().push_str(&trimmed);
                    }
                    (_, _) => {}
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
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
