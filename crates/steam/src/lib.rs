//! Steam for builds shipped on Steam: other apps' install folders and publishing Workshop items.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use steamworks::{AppId, Client, FileType, PublishedFileId, PublishedFileVisibility, SteamError, UpdateStatus, UpdateWatchHandle};

/// Steam's limits on an item's details, in bytes.
pub const TITLE_MAX: usize = 128;
pub const DESCRIPTION_MAX: usize = 8000;
pub const TAG_MAX: usize = 255;
pub const PREVIEW_MAX: u64 = 1024 * 1024;

/// The running Steam client.
pub struct Steam {
    client: Client,
}

impl Steam {
    /// Connects to Steam, which works when Steam started this program or a `steam_appid.txt` names the app.
    pub fn init() -> Result<Steam, String> {
        Client::init().map(|client| Steam { client }).map_err(|e| e.to_string())
    }

    /// Runs Steam's pending callbacks; call it every frame.
    pub fn pump(&self) {
        self.client.run_callbacks();
    }

    /// Where Steam installed `app`, when it is installed.
    pub fn app_dir(&self, app: u32) -> Option<PathBuf> {
        let apps = self.client.apps();
        if !apps.is_app_installed(AppId(app)) {
            return None;
        }
        let dir = apps.app_install_dir(AppId(app));
        (!dir.is_empty()).then(|| PathBuf::from(dir))
    }

    /// Starts creating or updating a Workshop item of `app`; follow it with [`Steam::poll`].
    pub fn publish(&self, app: u32, upload: Upload) -> Result<Job, String> {
        validate(&upload)?;
        let (tx, rx) = channel();
        let mut job = Job { app: AppId(app), upload, tx, rx, watch: None, needs_agreement: false };
        match job.upload.item {
            Some(id) => self.submit(&mut job, PublishedFileId(id))?,
            None => {
                let tx = job.tx.clone();
                self.client.ugc().create_item(job.app, FileType::Community, move |r| {
                    let _ = tx.send(r);
                });
            }
        }
        Ok(job)
    }

    /// Where `job` is; call it every frame until it is [`State::Done`].
    pub fn poll(&self, job: &mut Job) -> State {
        match job.rx.try_recv() {
            Ok(Ok((id, needs))) => {
                job.needs_agreement |= needs;
                job.upload.item = Some(id.0);
                if job.watch.is_some() {
                    return State::Done(Ok(Published { id: id.0, needs_agreement: job.needs_agreement }));
                }
                if let Err(e) = self.submit(job, id) {
                    return State::Done(Err(e));
                }
                State::Working { stage: "Uploading", done: 0, total: 0 }
            }
            Ok(Err(e)) => State::Done(Err(message(e))),
            Err(TryRecvError::Disconnected) => State::Done(Err("Steam stopped answering".into())),
            Err(TryRecvError::Empty) => match &job.watch {
                None => State::Working { stage: "Creating the item", done: 0, total: 0 },
                Some(w) => {
                    let (status, done, total) = w.progress();
                    let stage = match status {
                        UpdateStatus::UploadingContent => "Uploading the map",
                        UpdateStatus::UploadingPreviewFile => "Uploading the preview",
                        UpdateStatus::CommittingChanges => "Saving",
                        _ => "Preparing",
                    };
                    State::Working { stage, done, total }
                }
            },
        }
    }

    fn submit(&self, job: &mut Job, id: PublishedFileId) -> Result<(), String> {
        let (u, tx) = (&job.upload, job.tx.clone());
        let ugc = self.client.ugc();
        // steamworks asserts on any detail Steam refuses, so a refusal must not take the editor down.
        let watch = catch_unwind(AssertUnwindSafe(|| {
            let mut update = ugc
                .start_item_update(job.app, id)
                .title(&u.title)
                .description(&u.description)
                .content_path(&u.content)
                .visibility(u.visibility.into())
                .tags(u.tags.clone(), false);
            if let Some(p) = &u.preview {
                update = update.preview_path(p);
            }
            update.submit(Some(&u.change_note), move |r| {
                let _ = tx.send(r);
            })
        }))
        .map_err(|_| "Steam refused the item's details".to_string())?;
        job.watch = Some(watch);
        Ok(())
    }
}

/// What to publish: a folder of files and the details Steam shows.
#[derive(Clone, Debug, Default)]
pub struct Upload {
    /// The item to update; `None` creates one.
    pub item: Option<u64>,
    pub title: String,
    pub description: String,
    pub content: PathBuf,
    /// JPEG, PNG or GIF under [`PREVIEW_MAX`]; `None` keeps an update's preview.
    pub preview: Option<PathBuf>,
    pub tags: Vec<String>,
    pub visibility: Visibility,
    pub change_note: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    Public,
    FriendsOnly,
    Unlisted,
    #[default]
    Private,
}

impl Visibility {
    pub const ALL: [Visibility; 4] = [Visibility::Public, Visibility::FriendsOnly, Visibility::Unlisted, Visibility::Private];

    pub fn label(self) -> &'static str {
        match self {
            Visibility::Public => "Public",
            Visibility::FriendsOnly => "Friends only",
            Visibility::Unlisted => "Unlisted",
            Visibility::Private => "Private",
        }
    }
}

impl From<Visibility> for PublishedFileVisibility {
    fn from(v: Visibility) -> Self {
        match v {
            Visibility::Public => PublishedFileVisibility::Public,
            Visibility::FriendsOnly => PublishedFileVisibility::FriendsOnly,
            Visibility::Unlisted => PublishedFileVisibility::Unlisted,
            Visibility::Private => PublishedFileVisibility::Private,
        }
    }
}

/// A publish in progress.
pub struct Job {
    app: AppId,
    upload: Upload,
    tx: Sender<Result<(PublishedFileId, bool), SteamError>>,
    rx: Receiver<Result<(PublishedFileId, bool), SteamError>>,
    watch: Option<UpdateWatchHandle>,
    needs_agreement: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Working { stage: &'static str, done: u64, total: u64 },
    Done(Result<Published, String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Published {
    pub id: u64,
    /// The item stays hidden until its author accepts the Workshop agreement on its page.
    pub needs_agreement: bool,
}

/// The item's Steam page, which also asks for the Workshop agreement.
pub fn item_url(id: u64) -> String {
    format!("steam://url/CommunityFilePage/{id}")
}

/// Checks `u` against Steam's limits before any of it reaches Steam.
pub fn validate(u: &Upload) -> Result<(), String> {
    let text = |name: &str, s: &str, max: usize| {
        if s.contains('\0') {
            Err(format!("The {name} holds a character Steam can't take."))
        } else if s.len() > max {
            Err(format!("The {name} is {} bytes; Steam takes at most {max}.", s.len()))
        } else {
            Ok(())
        }
    };
    if u.title.trim().is_empty() {
        return Err("The item needs a title.".into());
    }
    text("title", &u.title, TITLE_MAX)?;
    text("description", &u.description, DESCRIPTION_MAX)?;
    text("change note", &u.change_note, DESCRIPTION_MAX)?;
    for tag in &u.tags {
        if tag.is_empty() || tag.len() > TAG_MAX || !tag.bytes().all(|b| (b' '..=b'~').contains(&b) && b != b',') {
            return Err(format!("Steam can't take the tag \"{tag}\"."));
        }
    }
    if !u.content.is_dir() {
        return Err(format!("{} is not a folder.", u.content.display()));
    }
    if let Some(p) = &u.preview {
        let len = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?.len();
        if len >= PREVIEW_MAX {
            return Err(format!("The preview is {} KB; Steam takes less than 1 MB.", len / 1024));
        }
        if !matches!(extension(p).as_deref(), Some("jpg" | "jpeg" | "png" | "gif")) {
            return Err("The preview must be a JPEG, PNG or GIF.".into());
        }
    }
    Ok(())
}

fn extension(p: &Path) -> Option<String> {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
}

fn message(e: SteamError) -> String {
    match e {
        SteamError::AccessDenied | SteamError::InsufficientPrivilege => {
            format!("Steam refused ({e}). Publishing for a game needs a Steam account that owns it.")
        }
        e => format!("Steam: {e}"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn validate_checks_steams_limits() {
        let dir = tempfile::tempdir().unwrap();
        let ok = Upload { title: "Surf".into(), content: dir.path().into(), tags: vec!["Tier 1".into()], ..Default::default() };
        assert_eq!(validate(&ok), Ok(()));
        assert!(validate(&Upload { title: "x".repeat(TITLE_MAX + 1), ..ok.clone() }).is_err());
        assert!(validate(&Upload { title: " ".into(), ..ok.clone() }).is_err());
        assert!(validate(&Upload { tags: vec!["a,b".into()], ..ok.clone() }).is_err());
        assert!(validate(&Upload { content: dir.path().join("missing"), ..ok.clone() }).is_err());
        let big = dir.path().join("big.jpg");
        std::fs::write(&big, vec![0u8; PREVIEW_MAX as usize + 100 * 1024]).unwrap();
        assert!(validate(&Upload { preview: Some(big), ..ok.clone() }).is_err());
        let small = dir.path().join("small.png");
        std::fs::write(&small, b"png").unwrap();
        assert_eq!(validate(&Upload { preview: Some(small), ..ok }), Ok(()));
    }
}
