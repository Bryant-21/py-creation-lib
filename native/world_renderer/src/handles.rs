use crate::error::{Result, WorldRendererError};
use crate::model::{WorldScene, WorldSession};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

#[derive(Default)]
pub struct HandleRegistry {
    next_id: AtomicU64,
    sessions: RwLock<HashMap<u64, Arc<WorldSession>>>,
    scenes: RwLock<HashMap<u64, Arc<WorldScene>>>,
}

impl HandleRegistry {
    pub fn insert_session(&self, session: WorldSession) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.sessions
            .write()
            .expect("session registry poisoned")
            .insert(id, Arc::new(session));
        id
    }

    pub fn with_session<T>(&self, id: u64, f: impl FnOnce(&WorldSession) -> T) -> Result<T> {
        let sessions = self.sessions.read().expect("session registry poisoned");
        let session = sessions.get(&id).ok_or(WorldRendererError::MissingHandle {
            kind: "session",
            id,
        })?;
        Ok(f(session))
    }

    pub fn remove_session(&self, id: u64) -> Result<()> {
        let removed = self
            .sessions
            .write()
            .expect("session registry poisoned")
            .remove(&id);
        removed
            .map(|_| ())
            .ok_or(WorldRendererError::MissingHandle {
                kind: "session",
                id,
            })
    }

    pub fn insert_scene(&self, scene: WorldScene) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.scenes
            .write()
            .expect("scene registry poisoned")
            .insert(id, Arc::new(scene));
        id
    }

    pub fn with_scene<T>(&self, id: u64, f: impl FnOnce(&WorldScene) -> T) -> Result<T> {
        let scenes = self.scenes.read().expect("scene registry poisoned");
        let scene = scenes
            .get(&id)
            .ok_or(WorldRendererError::MissingHandle { kind: "scene", id })?;
        Ok(f(scene))
    }

    pub fn remove_scene(&self, id: u64) -> Result<()> {
        let removed = self
            .scenes
            .write()
            .expect("scene registry poisoned")
            .remove(&id);
        removed
            .map(|_| ())
            .ok_or(WorldRendererError::MissingHandle { kind: "scene", id })
    }
}
