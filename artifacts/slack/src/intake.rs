//! Taking in one file of a speaker's post: download it with the bot token, decide its type from
//! its bytes, and keep it under its channel, or say why not.
//!
//! The checks run cheapest first, and nothing is stored until every one has passed: the channel
//! takes files at all, Slack can serve this one from its own file host, the size Slack reports
//! is within the limit, the bytes downloaded are within it too (the download asks for one byte
//! past the limit, so a size Slack under-reported can't fill the disk), and the type sniffed
//! from the bytes is one the channel lists. The filename and the type Slack reports are never
//! read.

use std::io;

use ikigai_core::ContentId;

use crate::{sniff, FileEntry, Files, Objects, SlackFile, SlackTransport};

/// The channel lists no file types.
pub const FILES_NOT_ACCEPTED: &str = "files-not-accepted";
/// Slack could not serve the file (no private URL, one off Slack's file host, or a download
/// that failed), or the door could not keep it.
pub const FILE_UNAVAILABLE: &str = "file-unavailable";
/// The file is larger than the channel's limit, by Slack's report or by its bytes.
pub const FILE_TOO_LARGE: &str = "file-too-large";
/// The bytes are not one of the types the channel lists.
pub const FILE_TYPE_REFUSED: &str = "file-type-refused";

/// Where Slack serves private files from. The bot token is sent to nothing else.
const FILE_HOST: &str = "https://files.slack.com/";

/// The files a channel accepts: the media types, and the most bytes one file may have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepts {
    pub types: Vec<String>,
    pub max_bytes: u64,
}

/// Takes in `file` for `channel`: its resource name, `urn:slack:channel:{channel}:file:b3:{hex}`,
/// once it is kept, or the reason it was refused. A file the store can't keep is noted as
/// `file-unavailable`, so one file never loses the post it came with.
pub(crate) fn take<T: SlackTransport>(
    transport: &mut T,
    objects: &Objects,
    files: &Files,
    channel: &str,
    accepts: Option<&Accepts>,
    file: &SlackFile,
) -> io::Result<String> {
    let Some(accepts) = accepts.filter(|a| !a.types.is_empty()) else {
        return Ok(FILES_NOT_ACCEPTED.to_owned());
    };
    if !file.url_private.starts_with(FILE_HOST) {
        return Ok(FILE_UNAVAILABLE.to_owned());
    }
    if file.size > accepts.max_bytes {
        return Ok(FILE_TOO_LARGE.to_owned());
    }
    let Ok(bytes) = transport.download(&file.url_private, accepts.max_bytes.saturating_add(1))
    else {
        return Ok(FILE_UNAVAILABLE.to_owned());
    };
    if bytes.len() as u64 > accepts.max_bytes {
        return Ok(FILE_TOO_LARGE.to_owned());
    }
    let Some(content_type) = sniff(&bytes).filter(|t| accepts.types.iter().any(|a| a == t)) else {
        return Ok(FILE_TYPE_REFUSED.to_owned());
    };
    // A store that can't keep the file loses that file, never the post: it is noted as
    // unavailable, like a download that failed, and the cause is logged.
    Ok(keep(objects, files, channel, file, content_type, &bytes).unwrap_or_else(|e| {
        eprintln!("file {:?} in {channel} not kept: {e}", file.id);
        FILE_UNAVAILABLE.to_owned()
    }))
}

/// Keeps `bytes`, sniffed as `content_type`, in the object store and names them in `channel`'s
/// index: the resource name.
fn keep(
    objects: &Objects,
    files: &Files,
    channel: &str,
    file: &SlackFile,
    content_type: &str,
    bytes: &[u8],
) -> io::Result<String> {
    let stored_before = objects.path(&ContentId::of(bytes)).is_file();
    let id = objects.put(bytes)?;
    // The index names the file by Slack's id for it, with anything but letters and digits
    // dropped, so nothing Slack sent can break an index line.
    let slack_id = file.id.chars().filter(char::is_ascii_alphanumeric).collect();
    let entry = FileEntry { id: id.clone(), content_type: content_type.to_owned(), name: slack_id };
    // Bytes this take stored but no index names are removed again, so none outlive the
    // retention chore, which finds an object only through a name. Bytes stored before stay.
    files.add(channel, entry).inspect_err(|_| {
        if !stored_before {
            let _ = objects.remove(&id);
        }
    })?;
    Ok(format!("urn:slack:channel:{channel}:file:{id}"))
}
