use html5ever::tokenizer::TokenSink;

pub mod czbooksnet;

pub trait Provider {
    type Link: Default + TokenSink<Handle = ()> + Into<BookInfo>;
    type Chapter: Default + TokenSink<Handle = ()> + Into<ChapterInfo>;
}

pub struct BookInfo {
    pub author: String,
    pub title: String,
    pub links: Vec<String>,
}

pub struct ChapterInfo {
    pub title: String,
    pub text: String,
}
