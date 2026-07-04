// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Licensed under either the MIT License or the Apache License 2.0.
// See the LICENSE-MIT or LICENSE-APACHE file in this repository:
// https://github.com/ankit-chaubey/ferogram
//
// Feel free to use, modify, and share this code.
// Please keep this notice when redistributing.

//! Fetch the admin action log of a supergroup or channel.
//!
//! The admin log records every moderation action - bans, kicks, message
//! deletions, title changes, pinned messages, permission edits - with the
//! acting admin's user ID and a Unix timestamp. This is only accessible via
//! MTProto; the Bot API has no equivalent endpoint.
//!
//! Run:
//!   cargo run --example admin_log
//!
//! You must be an admin of the target group/channel to read the log.
//! Fill in API_ID, API_HASH, PHONE and GROUP below.

use chrono::{TimeZone, Utc};
use ferogram::tl;
use ferogram::{Client, TransportKind};

const API_ID: i32 = 0; // from https://my.telegram.org
const API_HASH: &str = ""; // from https://my.telegram.org
const PHONE: &str = ""; // your phone number, e.g. "+15551234567"

/// Username or invite link of the supergroup/channel to inspect.
/// Must be a supergroup or broadcast channel (not a basic group).
/// Example: "@my_supergroup" or "-1001234567890"
const GROUP: &str = "@your_supergroup";

/// Number of events to fetch (max 100 per call).
const LIMIT: i32 = 20;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if API_ID == 0
        || API_HASH.is_empty()
        || PHONE.is_empty()
        || GROUP.starts_with('@') && GROUP == "@your_supergroup"
    {
        eprintln!("Fill in API_ID, API_HASH, PHONE and GROUP at the top of admin_log.rs");
        std::process::exit(1);
    }

    println!("Connecting...");
    let (client, _shutdown) = Client::builder()
        .api_id(API_ID)
        .api_hash(API_HASH)
        .transport(TransportKind::Abridged)
        .connect()
        .await?;

    if !client.is_authorized().await? {
        login(&client).await?;
        client.save_session().await?;
        println!("Session saved.");
    }

    let me = client.get_me().await?;
    let display = me
        .first_name
        .as_deref()
        .unwrap_or(me.username.as_deref().unwrap_or("?"));
    println!("Logged in as {display}\n");

    println!(
        "Admin log for {GROUP} (last {LIMIT} events):\n{}",
        "-".repeat(60)
    );

    // get_admin_log(peer, search_query, limit, max_id, min_id)
    // Pass "" for the query to return all event types.
    // max_id=0 / min_id=0 means no bounding - start from the newest event.
    let events = client.get_admin_log(GROUP, "", LIMIT, 0, 0).await?;

    if events.is_empty() {
        println!("No events found (log may be empty or you may lack admin access).");
        return Ok(());
    }

    for ev in &events {
        let ts = Utc
            .timestamp_opt(ev.date as i64, 0)
            .single()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| format!("unix={}", ev.date));

        let action_name = describe_action(&ev.action);
        println!(
            "[{:>12}] user_id={:<12} {ts}  {action_name}",
            ev.id, ev.user_id
        );
    }

    println!("{}\nFetched {} event(s).", "-".repeat(60), events.len());
    Ok(())
}

/// Return a short human-readable label for an admin log event action.
fn describe_action(action: &tl::enums::ChannelAdminLogEventAction) -> &'static str {
    use tl::enums::ChannelAdminLogEventAction as A;
    match action {
        A::ParticipantJoin => "joined",
        A::ParticipantLeave => "left",
        A::ParticipantInvite(_) => "invited a user",
        A::ParticipantToggleBan(_) => "banned/unbanned a user",
        A::ParticipantToggleAdmin(_) => "changed admin rights",
        A::ParticipantJoinByInvite(_) => "joined via invite link",
        A::ParticipantJoinByRequest(_) => "approved join request",
        A::ParticipantMute(_) => "muted a participant",
        A::ParticipantUnmute(_) => "unmuted a participant",
        A::ParticipantVolume(_) => "changed participant volume",
        A::ParticipantSubExtend(_) => "extended subscription",
        A::ChangeTitle(_) => "changed title",
        A::ChangeAbout(_) => "changed description",
        A::ChangeUsername(_) => "changed username",
        A::ChangeUsernames(_) => "changed usernames",
        A::ChangePhoto(_) => "changed photo",
        A::ChangeStickerSet(_) => "changed sticker set",
        A::ChangeEmojiStickerSet(_) => "changed emoji sticker set",
        A::ChangeEmojiStatus(_) => "changed emoji status",
        A::ChangeLinkedChat(_) => "changed linked chat",
        A::ChangeLocation(_) => "changed location",
        A::ChangeHistoryTtl(_) => "changed message TTL",
        A::ChangeAvailableReactions(_) => "changed reactions",
        A::ChangePeerColor(_) => "changed peer color",
        A::ChangeProfilePeerColor(_) => "changed profile color",
        A::ChangeWallpaper(_) => "changed wallpaper",
        A::ToggleInvites(_) => "toggled invites",
        A::ToggleSignatures(_) => "toggled signatures",
        A::ToggleSignatureProfiles(_) => "toggled signature profiles",
        A::TogglePreHistoryHidden(_) => "toggled pre-history visibility",
        A::ToggleSlowMode(_) => "toggled slow mode",
        A::ToggleGroupCallSetting(_) => "changed group call setting",
        A::ToggleNoForwards(_) => "toggled no-forward restriction",
        A::ToggleForum(_) => "toggled forum mode",
        A::ToggleAntiSpam(_) => "toggled anti-spam",
        A::ToggleAutotranslation(_) => "toggled auto-translation",
        A::UpdatePinned(_) => "pinned/unpinned message",
        A::EditMessage(_) => "edited a message",
        A::DeleteMessage(_) => "deleted a message",
        A::DefaultBannedRights(_) => "changed default banned rights",
        A::StopPoll(_) => "stopped a poll",
        A::StartGroupCall(_) => "started group call",
        A::DiscardGroupCall(_) => "ended group call",
        A::ExportedInviteDelete(_) => "deleted invite link",
        A::ExportedInviteRevoke(_) => "revoked invite link",
        A::ExportedInviteEdit(_) => "edited invite link",
        A::SendMessage(_) => "sent a message",
        A::CreateTopic(_) => "created topic",
        A::EditTopic(_) => "edited topic",
        A::DeleteTopic(_) => "deleted topic",
        A::PinTopic(_) => "pinned topic",
        A::ParticipantEditRank(_) => "changed participant rank",
    }
}

async fn login(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    let name = client.interactive_sign_in(PHONE).await?;
    println!("Signed in as {name}");
    Ok(())
}
