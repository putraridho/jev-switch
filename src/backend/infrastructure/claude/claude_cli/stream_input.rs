//! Stream-json input: the user's message as one JSON line, with images as blocks.

use super::ClaudeBackend;
use crate::backend::{ProviderId, TurnRequest};

/// One line of stream-json input: the user's message.
#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum InputLine<'a> {
    User { message: InputMessage<'a> },
}

#[derive(serde::Serialize)]
struct InputMessage<'a> {
    role: Role,
    content: InputContent<'a>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    User,
}

/// A plain prompt is a string; one with images is a list of blocks.
#[derive(serde::Serialize)]
#[serde(untagged)]
enum InputContent<'a> {
    Text(&'a str),
    Blocks(Vec<InputBlock<'a>>),
}

#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum InputBlock<'a> {
    Image { source: ImageSource },
    Text { text: &'a str },
}

#[derive(serde::Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    encoding: Encoding,
    media_type: MediaType,
    data: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum Encoding {
    Base64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(super) enum MediaType {
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/gif")]
    Gif,
    #[serde(rename = "image/webp")]
    Webp,
}

impl MediaType {
    /// The type the file's first bytes say, whatever its name: the API refuses an image
    /// whose declared type is wrong (a JPEG saved as `.png`).
    pub(super) fn sniff(head: &[u8]) -> Option<Self> {
        if head.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if head.starts_with(&[0xff, 0xd8, 0xff]) {
            Some(Self::Jpeg)
        } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
            Some(Self::Gif)
        } else if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
            Some(Self::Webp)
        } else {
            None
        }
    }
}

/// Whether a request's images go as image blocks: Claude takes them; DeepSeek's API does
/// not, so it gets their paths in the prompt instead.
pub(super) fn sends_image_blocks(provider: ProviderId, req: &TurnRequest) -> bool {
    !req.images.is_empty() && provider != ProviderId::DeepSeek
}

/// The largest image sent; bigger ones are left out with a note in the prompt. The API's
/// 5 MB limit counts the base64 text, which is 4/3 of the file.
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024 * 3 / 4;

/// One image block, or the note that says why it was left out.
fn image_block(path: &std::path::Path) -> Result<ImageSource, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Could not read image {}: {e}", path.display()))?.len();
    if size > MAX_IMAGE_BYTES {
        return Err(format!("Left out an image over 5 MB: {}", path.display()));
    }
    let data = std::fs::read(path).map_err(|e| format!("Could not read image {}: {e}", path.display()))?;
    let media_type = MediaType::sniff(&data).ok_or_else(|| format!("Left out a file that is not a PNG, JPEG, GIF or WebP image: {}", path.display()))?;
    Ok(ImageSource { encoding: Encoding::Base64, media_type, data: crate::shared::domain::base64::encode(&data) })
}

/// The image blocks of a request, and the prompt with a note for each image left out.
fn image_content(req: &TurnRequest) -> (Vec<InputBlock<'static>>, String) {
    let mut blocks = Vec::new();
    let mut prompt = req.prompt.clone();
    for path in &req.images {
        let path = if path.is_absolute() { path.clone() } else { req.dir.join(path) };
        match image_block(&path) {
            Ok(source) => blocks.push(InputBlock::Image { source }),
            Err(note) => prompt.push_str(&format!("\n({note})")),
        }
    }
    (blocks, prompt)
}

impl ClaudeBackend {
    /// Stream-json input is one JSON message per line, not the bare prompt.
    pub(super) fn stdin_message(&self, req: &TurnRequest) -> Option<String> {
        let blocks_wanted = sends_image_blocks(self.provider, req);
        if !self.interactive && !blocks_wanted {
            return Some(crate::backend::prompt_with_images(req));
        }
        let (mut blocks, prompt) = if blocks_wanted { image_content(req) } else { (Vec::new(), crate::backend::prompt_with_images(req)) };
        let content = if !blocks_wanted {
            InputContent::Text(&prompt)
        } else {
            blocks.push(InputBlock::Text { text: &prompt });
            InputContent::Blocks(blocks)
        };
        let line = InputLine::User { message: InputMessage { role: Role::User, content } };
        serde_json::to_string(&line).ok().map(|s| format!("{s}\n"))
    }

    /// A message typed during the turn, as a stream-json line.
    pub(super) fn steer_message(&self, text: &str) -> Option<String> {
        let line = InputLine::User { message: InputMessage { role: Role::User, content: InputContent::Text(text) } };
        serde_json::to_string(&line).ok()
    }
}
