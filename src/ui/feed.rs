//! How a window keeps up with the daemon and with the clock.

use std::{sync::Arc, time::Duration};

use chrono::Local;
use crownui::prelude::Cx;
use tokio::sync::watch;

use crate::{daemon::Snapshot, ui::env::Env};

/// How often the relative timestamps are re-rendered.
const TICK: Duration = Duration::from_secs(1);
/// How often pictures nobody has shown since the last sweep leave the cache.
const ICON_SWEEP: Duration = Duration::from_secs(30);

/// Applies the daemon's current snapshot, then every one after it, until the
/// daemon goes away.
pub fn follow(
    cx: &mut Cx,
    mut snapshots: watch::Receiver<Arc<Snapshot>>,
    apply: impl Fn(&mut Cx, &Snapshot) + Copy + 'static,
) {
    let current = Arc::clone(&snapshots.borrow_and_update());
    apply(cx, &current);
    next(cx, snapshots, apply);
}

fn next(
    cx: &mut Cx,
    mut snapshots: watch::Receiver<Arc<Snapshot>>,
    apply: impl Fn(&mut Cx, &Snapshot) + Copy + 'static,
) {
    cx.spawn(
        async move {
            let alive = snapshots.changed().await.is_ok();
            (snapshots, alive)
        },
        move |cx, (mut snapshots, alive)| {
            if !alive {
                log::error!("the notification core stopped publishing");
                return;
            }
            let snapshot = Arc::clone(&snapshots.borrow_and_update());
            apply(cx, &snapshot);
            next(cx, snapshots, apply);
        },
    );
}

/// Keeps `env.now` current for the timestamps and sweeps the icon cache.
pub fn keep_time(cx: &mut Cx, env: Env) {
    tick(cx, env);
    sweep(cx, env);
}

fn tick(cx: &mut Cx, env: Env) {
    cx.spawn(async { tokio::time::sleep(TICK).await }, move |cx, ()| {
        env.now.set(cx, Local::now());
        tick(cx, env);
    });
}

fn sweep(cx: &mut Cx, env: Env) {
    cx.spawn(
        async { tokio::time::sleep(ICON_SWEEP).await },
        move |cx, ()| {
            env.icons.update(cx, |icons| icons.sweep());
            sweep(cx, env);
        },
    );
}
