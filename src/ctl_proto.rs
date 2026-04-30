use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Receiver, SyncSender, Sender};

use serde::{Deserialize, Serialize};

pub type ReplySender = SyncSender<Result<(), String>>;
pub type CommandSender = Sender<CtlCommand>;
pub type CommandReceiver = Receiver<CtlCommand>;

pub enum CtlCommand {
    Start   { service: String, reply: ReplySender },
    Restart { service: String, reply: ReplySender },
    Reload  { service: String, reply: ReplySender },
}

pub fn new_command_channel() -> (CommandSender, CommandReceiver) {
    mpsc::channel()
}

/// Per-service status as seen by the control socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub state: String, // "pending" | "running" | "ready" | "exited" | "failed"
    pub pid: Option<u32>,
}

/// Shared state between the executor and the ctl server.
pub type SharedState = Arc<Mutex<HashMap<String, ServiceInfo>>>;

pub fn new_shared_state() -> SharedState {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Commands sent from flint-ctl to flint-init.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    Status,
    Stop    { service: String },
    Start   { service: String },
    Restart { service: String },
    Reload  { service: String },
}

/// Responses sent from flint-init back to flint-ctl.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Status { services: Vec<ServiceInfo> },
    Ok { ok: bool },
    Error { error: String },
}
