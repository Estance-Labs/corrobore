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
pub mod app;
pub mod auth;
pub mod config;
pub mod correlation;
mod durability;
mod enterprise;
pub mod error;
pub mod explorer_timeline;
pub mod handlers;
pub mod lifecycle;
pub mod logging;
pub mod opencti_shadow;
pub mod opencti_sync;
pub mod opencti_write;
pub mod security;
pub mod session_runtime;
mod storage_ownership;
pub mod visualization;
mod web;

pub use app::{
    AppState, AppStateInitError, PersistentRuntimeStore, RuntimeStoreProvider, build_router,
};
pub use config::{ServerConfig, StorageMode};
pub use lifecycle::{
    LifecycleState, ServerLifecycle, ServerLifecycleError, install_shutdown_signal,
    serve_tls_with_lifecycle, serve_with_lifecycle,
};
