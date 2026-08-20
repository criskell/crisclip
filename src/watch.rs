use anyhow::{Context, Result};
use std::sync::mpsc::SyncSender;
use std::thread;
use x11rb::connection::Connection;
use x11rb::protocol::xfixes::{self, SelectionEventMask};
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::Event;

pub fn spawn(notify: SyncSender<()>) -> Result<()> {
    let (connection, screen) = x11rb::connect(None).context("cannot reach the X server")?;
    let root = connection.setup().roots[screen].root;

    xfixes::query_version(&connection, 5, 0)
        .context("XFIXES is missing")?
        .reply()
        .context("XFIXES is missing")?;

    let clipboard = connection.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
    xfixes::select_selection_input(
        &connection,
        root,
        clipboard,
        SelectionEventMask::SET_SELECTION_OWNER,
    )?;
    connection.flush()?;

    thread::spawn(move || {
        while let Ok(event) = connection.wait_for_event() {
            if matches!(event, Event::XfixesSelectionNotify(_)) {
                let _ = notify.try_send(());
            }
        }
    });

    Ok(())
}
