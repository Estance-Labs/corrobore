// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
use std::{collections::HashMap, fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::IngestError;

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedCursorState {
    cursors: HashMap<String, String>,
}
/// File-backed store for per-collection TAXII `added_after` cursors.
#[derive(Debug)]
pub struct CursorStore {
    state_file: PathBuf,
}

impl CursorStore {
    /// Creates a store rooted at `state_dir`.
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_file: state_dir.into().join("cursors.json"),
        }
    }

    /// Returns the persisted cursor for `collection_id`, if any.
    pub fn load_cursor(&self, collection_id: &str) -> Result<Option<String>, IngestError> {
        Ok(self.read_state()?.cursors.get(collection_id).cloned())
    }

    /// Persists `added_after` as the cursor for `collection_id`.
    pub fn save_cursor(
        &mut self,
        collection_id: &str,
        added_after: &str,
    ) -> Result<(), IngestError> {
        let mut state = self.read_state()?;
        state
            .cursors
            .insert(collection_id.to_owned(), added_after.to_owned());
        self.write_state(&state)
    }

    fn read_state(&self) -> Result<PersistedCursorState, IngestError> {
        if !self.state_file.exists() {
            return Ok(PersistedCursorState::default());
        }

        let bytes =
            fs::read(&self.state_file).map_err(|error| IngestError::State(error.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|error| IngestError::State(error.to_string()))
    }

    fn write_state(&self, state: &PersistedCursorState) -> Result<(), IngestError> {
        if let Some(parent) = self.state_file.parent() {
            fs::create_dir_all(parent).map_err(|error| IngestError::State(error.to_string()))?;
        }

        let payload = serde_json::to_vec_pretty(state)
            .map_err(|error| IngestError::State(error.to_string()))?;
        fs::write(&self.state_file, payload).map_err(|error| IngestError::State(error.to_string()))
    }
}
