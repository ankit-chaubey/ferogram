// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// Licensed under either the MIT License or the Apache License 2.0.

//! The sender task: a single `tokio::spawn`-ed loop that owns [`MtpSender`]
//! and is the only entity that touches the TCP socket.
//!
//! External callers interact via two channels:
//!
//! - [`RpcEnqueue`]: send a pre-serialised TL body + oneshot::Sender to the task.
//!   The task enqueues it into `MtpSender`, and the oneshot is fulfilled when the
//!   server responds.  This replaces the old `do_rpc_call` + `Mutex<ConnectionWriter>`
//!   + `pending` HashMap pattern.
//!
//! - [`ReconnectRequest`]: send a new `(TcpStream, EncryptedSession, FrameKind,
//!   Option<perm_key>)` to the task after a reconnect completes.  The task calls
//!   `MtpSender::set_stream` and resumes the loop.
//!
//! The task forwards raw update bodies (everything `MtpSender::step()` returns
//! that is not an rpc_result) via [`FrameEvent`] to the client's dispatch path.

use ferogram_connect::FrameKind;
use ferogram_mtproto::EncryptedSession;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

use crate::errors::InvocationError;
use crate::mtp_sender::MtpSender;

/// A single RPC request sent from any caller to the sender task.
pub struct RpcEnqueue {
    /// Pre-serialised TL body (output of `EncryptedSession::pack_body_with_msg_id`
    /// or any raw TL bytes; the sender task will re-encrypt via MtpSender).
    pub body: Vec<u8>,
    /// Fulfilled with the raw rpc_result body (or an error) when the server responds.
    pub tx: oneshot::Sender<Result<Vec<u8>, InvocationError>>,
}

/// Reconnect request: replace the TCP stream inside the sender task.
pub struct ReconnectRequest {
    pub stream: TcpStream,
    pub enc: EncryptedSession,
    pub frame_kind: FrameKind,
    pub perm_auth_key: Option<[u8; 256]>,
}

/// Events the sender task sends back to the client.
pub enum FrameEvent {
    /// A raw update body (Updates, UpdateShort, etc.) to dispatch.
    Update(Vec<u8>),
    /// The connection failed; the client must reconnect and send a ReconnectRequest.
    Error(InvocationError),
    /// Session info after initial connect or reconnect (for session saving).
    Connected {
        auth_key: [u8; 256],
        first_salt: i64,
        time_offset: i32,
        session_id: i64,
    },
}

/// Sender-side handles given to the client after spawning the sender task.
pub struct SenderHandle {
    /// Enqueue RPC requests here.
    pub rpc_tx: mpsc::Sender<RpcEnqueue>,
    /// Send a new stream here after reconnect.
    pub reconnect_tx: mpsc::Sender<ReconnectRequest>,
}

/// Spawn the sender task.  Returns a [`SenderHandle`] for the client and an
/// `mpsc::Receiver<FrameEvent>` for receiving update bodies and errors.
pub fn spawn_sender_task(
    stream: TcpStream,
    enc: EncryptedSession,
    frame_kind: FrameKind,
    perm_auth_key: Option<[u8; 256]>,
) -> (SenderHandle, mpsc::Receiver<FrameEvent>) {
    let (rpc_tx, rpc_rx) = mpsc::channel::<RpcEnqueue>(512);
    let (reconnect_tx, reconnect_rx) = mpsc::channel::<ReconnectRequest>(4);
    let (frame_tx, frame_rx) = mpsc::channel::<FrameEvent>(256);

    let sender = MtpSender::new(stream, enc, frame_kind, perm_auth_key);

    tokio::spawn(sender_loop(sender, rpc_rx, reconnect_rx, frame_tx));

    (
        SenderHandle {
            rpc_tx,
            reconnect_tx,
        },
        frame_rx,
    )
}

async fn sender_loop(
    mut sender: MtpSender,
    mut rpc_rx: mpsc::Receiver<RpcEnqueue>,
    mut reconnect_rx: mpsc::Receiver<ReconnectRequest>,
    frame_tx: mpsc::Sender<FrameEvent>,
) {
    // Notify the client that we are connected and ready.
    let _ = frame_tx
        .send(FrameEvent::Connected {
            auth_key: sender.auth_key_bytes(),
            first_salt: sender.first_salt(),
            time_offset: sender.time_offset(),
            session_id: sender.session_id(),
        })
        .await;

    loop {
        // Drain all pending RPC enqueues before stepping (non-blocking).
        loop {
            match rpc_rx.try_recv() {
                Ok(enqueue) => sender.enqueue(enqueue.body, enqueue.tx),
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    // Client dropped all handles: shut down cleanly.
                    return;
                }
            }
        }

        tokio::select! {
            biased;

            // New RPC enqueue arrived while we were waiting in step().
            Some(enqueue) = rpc_rx.recv() => {
                sender.enqueue(enqueue.body, enqueue.tx);
                // Loop back immediately so step() can send it.
                continue;
            }

            // Reconnect request: swap the stream.
            Some(req) = reconnect_rx.recv() => {
                tracing::debug!("[sender_task] reconnect: swapping stream");
                sender.set_stream(req.stream, req.enc, req.frame_kind, req.perm_auth_key);
                let _ = frame_tx
                    .send(FrameEvent::Connected {
                        auth_key: sender.auth_key_bytes(),
                        first_salt: sender.first_salt(),
                        time_offset: sender.time_offset(),
                        session_id: sender.session_id(),
                    })
                    .await;
                continue;
            }

            // Drive one network event.
            result = sender.step() => {
                match result {
                    Ok(updates) => {
                        for body in updates {
                            if frame_tx.send(FrameEvent::Update(body)).await.is_err() {
                                // Client gone.
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[sender_task] connection error: {e:?}");
                        // Fail all pending requests immediately.
                        sender.fail_all(&e);
                        // Notify the client; it will reconnect and send ReconnectRequest.
                        if frame_tx.send(FrameEvent::Error(e)).await.is_err() {
                            return;
                        }
                        // Wait for a reconnect before driving step() again.
                        match reconnect_rx.recv().await {
                            Some(req) => {
                                tracing::debug!("[sender_task] reconnect received after error");
                                sender.set_stream(
                                    req.stream,
                                    req.enc,
                                    req.frame_kind,
                                    req.perm_auth_key,
                                );
                                let _ = frame_tx
                                    .send(FrameEvent::Connected {
                                        auth_key: sender.auth_key_bytes(),
                                        first_salt: sender.first_salt(),
                                        time_offset: sender.time_offset(),
                                        session_id: sender.session_id(),
                                    })
                                    .await;
                            }
                            None => {
                                // Client dropped reconnect handle: shut down.
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
}
