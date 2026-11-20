use log::*;

pub mod folder;
pub mod playlist;
pub mod rundown;

pub use playlist::CurrentProgram;
pub use rundown::RundownSource;

use crate::{
    player::{controller::ChannelManager, input::folder::FolderSource, utils::Media},
    utils::config::ProcessMode::*,
};

pub enum SourceIterator {
    Folder(Box<FolderSource>),
    Playlist(Box<CurrentProgram>),
    Rundown(Box<RundownSource>),
}

impl SourceIterator {
    pub async fn next(&mut self) -> Option<Media> {
        match self {
            SourceIterator::Folder(folder_source) => folder_source.next().await,
            SourceIterator::Playlist(program) => program.next().await,
            SourceIterator::Rundown(rundown_source) => rundown_source.next().await,
        }
    }
}

/// Create a source iterator from playlist, rundown queue, or folder.
pub async fn source_generator(manager: ChannelManager) -> SourceIterator {
    let config = manager.config.read().await.clone();
    let id = config.general.channel_id;
    let is_alive = manager.is_alive.clone();
    let current_list = manager.current_list.clone();

    match config.processing.mode {
        Folder => {
            info!(channel = id; "Playout in folder mode");
            let config_clone = config.clone();

            manager
                .storage
                .watchman(config_clone, is_alive, current_list)
                .await;

            let folder_source = FolderSource::new(&config, manager);

            SourceIterator::Folder(Box::new(folder_source.await))
        }
        Playlist => {
            info!(channel = id; "Playout in playlist mode");
            let program = CurrentProgram::new(manager);

            SourceIterator::Playlist(Box::new(program.await))
        }
        Rundown => {
            let rundown_source = RundownSource::new(manager).await;

            SourceIterator::Rundown(Box::new(rundown_source))
        }
    }
}
