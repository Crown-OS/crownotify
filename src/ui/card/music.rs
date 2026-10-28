//! The now-playing card: album art, track, scrubber, transport.
//!
//! This is a *cross-device* control surface. The track it shows is very often
//! playing on a phone that crownconnect is relaying, so nothing here may assume a
//! local player: the card renders entirely from the
//! [`Music`](crate::model::Music) payload it was handed — including whether the
//! transport is running — and it draws the same way whether the daemon can
//! reach that player or not. What it produces is a
//! [`MediaAction`](crate::ui::hit::MediaAction), which the surface layer turns
//! into a crownconnect command; if nothing is listening, the card simply keeps
//! showing what it was last told.
//!
//! The scrubber is the one control whose meaning depends on *where* it was
//! clicked, so the region it pushes carries the current position as a
//! placeholder and [`resolve_hit`](crate::ui::card::resolve_hit) rewrites it
//! from the click.

use crownshell::{
    Text, TextContext,
    kurbo::{Point, Rect},
    peniko::Color,
};

use crate::{
    model::{Content, DeviceKind, Icon, Notification, NotificationId},
    profile::ContentHint,
    ui::{
        card::{
            CONTROL_GAP, CONTROL_SPACING, LINE_GAP, MIN_COLUMN, MeasureCtx, RenderCtx, Row,
            content_rect, draw_icon, set_text, view::CardView,
        },
        hit::{CardAction, MediaAction},
        theme::Theme,
        widget::{IconButton, Symbol, draw_scrubber, percent_fraction, sync_style},
    },
};

/// How much bigger the album art is than an app icon. Cover art is the card's
/// only real ornament, so it earns the extra pixels the transport row costs.
const ART_SCALE: f64 = 1.3;
/// Corner radius of the album art, as a fraction of its side. Softer than an
/// app icon's, because a square photograph with sharp corners reads as an
/// un-styled image rather than as part of the card.
const ART_RADIUS_RATIO: f64 = 0.16;
/// Gap above the scrubber.
const SCRUB_GAP: f64 = 10.0;
/// Height of the band the scrubber's track is centred in. The band, not the
/// track, is the pointer target — a 4 px seek target is unusable.
const SCRUB_BAND: f64 = 14.0;

/// The music card's retained state.
pub struct MusicView {
    art: Option<Icon>,
    device: Option<DeviceKind>,
    title: Text,
    artist: Text,
    playing: bool,
    percent: u8,
}

impl MusicView {
    pub fn new(notification: &Notification) -> Self {
        let mut view = Self {
            art: None,
            device: None,
            title: Text::new(""),
            artist: Text::new(""),
            playing: false,
            percent: 0,
        };
        view.sync(notification);
        view
    }

    /// The three transport buttons, centred along the bottom of the card.
    fn render_transport(&mut self, id: NotificationId, ctx: &mut RenderCtx<'_>) {
        let metrics = ctx.theme.metrics;
        let neutral = (ctx.theme.palette.field, ctx.theme.palette.primary);
        let accent = ctx.accent();
        let middle = ctx.bounds.y1 - metrics.pad_y - metrics.button / 2.0;
        let center_x = ctx.bounds.center().x;
        let step = metrics.button + CONTROL_SPACING * 2.0;

        let toggle = if self.playing {
            (Symbol::Pause, MediaAction::Pause)
        } else {
            (Symbol::Play, MediaAction::Play)
        };

        let controls = [
            (
                Symbol::Previous,
                MediaAction::Previous,
                neutral,
                center_x - step,
            ),
            (toggle.0, toggle.1, (accent, Color::WHITE), center_x),
            (Symbol::Next, MediaAction::Next, neutral, center_x + step),
        ];

        for (symbol, media, (fill, glyph), x) in controls {
            let button = IconButton::new(symbol, fill, glyph, metrics.button);
            let center = Point::new(x, middle);
            let action = CardAction::Media(media);
            let state = ctx.state(&action);
            button.render(ctx.scene, center, state, 1.0);
            ctx.hits.push(button.bounds(center), id, action);
        }
    }

    fn rows(&mut self, column: f64, tcx: &mut TextContext) -> Column {
        self.title.set_max_width(Some(column));
        self.title.set_max_lines(Some(1));
        self.artist.set_max_width(Some(column));
        self.artist.set_max_lines(Some(1));

        let title = Row {
            top: 0.0,
            height: self.title.height(tcx),
        };
        let mut y = title.height;
        let artist = if self.artist.text().is_empty() {
            Row::default()
        } else {
            y += LINE_GAP;
            let row = Row {
                top: y,
                height: self.artist.height(tcx),
            };
            y += row.height;
            row
        };

        y += SCRUB_GAP;
        let scrubber = Row {
            top: y,
            height: SCRUB_BAND,
        };
        y += scrubber.height;

        Column {
            title,
            artist,
            scrubber,
            height: y,
        }
    }
}

impl CardView for MusicView {
    fn kind(&self) -> ContentHint {
        ContentHint::Music
    }

    /// Re-reads `notification`, reporting whether the card's height could have
    /// moved as a result.
    ///
    /// A track advancing is by far the most frequent replacement this daemon
    /// sees, and it moves neither the title nor the artist — so `percent` and
    /// `playing` deliberately do not count as a change, and a scrubbing card
    /// never re-measures.
    fn sync(&mut self, notification: &Notification) -> bool {
        let music = match &notification.content {
            Content::Music(music) => Some(music),
            _ => None,
        };

        let title = music.map_or(notification.summary.as_str(), |music| music.title.as_str());
        let artist = music.map_or(notification.body.as_str(), |music| music.artist.as_str());
        let changed = set_text(&mut self.title, title) | set_text(&mut self.artist, artist);

        self.playing = music.is_some_and(|music| music.playing);
        self.percent = music.map_or(0, |music| music.percent);

        let art = music
            .and_then(|music| music.album_art.as_ref())
            .or(notification.image.as_ref())
            .or(notification.app.icon.as_ref());
        if self.art.as_ref() != art {
            self.art = art.cloned();
        }
        self.device = notification.relayed_from.as_ref().map(|device| device.kind);
        changed
    }
    fn apply_theme(&mut self, theme: &Theme) {
        sync_style(&mut self.title, &theme.text.summary);
        sync_style(&mut self.artist, &theme.text.body);
    }
    fn measure(&mut self, width: f64, ctx: &mut MeasureCtx<'_>) -> f64 {
        let metrics = &ctx.theme.metrics;
        let art = metrics.icon * ART_SCALE;
        let column = (width - 2.0 * metrics.pad_x - art - metrics.icon_gap).max(MIN_COLUMN);
        let rows = self.rows(column, ctx.text);
        2.0 * metrics.pad_y + rows.height.max(art) + CONTROL_GAP + metrics.button
    }
    fn render(&mut self, id: NotificationId, ctx: &mut RenderCtx<'_>) {
        let metrics = ctx.theme.metrics;
        let inner = content_rect(ctx.bounds, ctx.theme);
        let art = metrics.icon * ART_SCALE;

        let art_rect = Rect::new(inner.x0, inner.y0, inner.x0 + art, inner.y0 + art);
        draw_icon(
            ctx,
            self.art.as_ref(),
            art_rect,
            art * ART_RADIUS_RATIO,
            self.device,
        );

        let left = art_rect.x1 + metrics.icon_gap;
        let column = (inner.x1 - left).max(MIN_COLUMN);
        let rows = self.rows(column, ctx.text);
        let top = inner.y0;

        for (row, text) in [
            (rows.title, &mut self.title),
            (rows.artist, &mut self.artist),
        ] {
            if row.shown() {
                text.draw(ctx.text, ctx.scene, (left.round(), (top + row.top).round()));
            }
        }

        let band = Rect::new(
            left,
            top + rows.scrubber.top,
            inner.x1,
            top + rows.scrubber.top + rows.scrubber.height,
        );
        // The placeholder percentage is the play head's own position, so a
        // caller that forgets to `resolve_hit` seeks to where the track already
        // is rather than to zero.
        let seek = CardAction::Media(MediaAction::Seek(self.percent));
        let active = matches!(ctx.hovered, Some(CardAction::Media(MediaAction::Seek(_))));
        let theme = ctx.theme;
        draw_scrubber(
            ctx.scene,
            band,
            percent_fraction(self.percent),
            theme,
            1.0,
            active,
        );
        ctx.hits.push(band, id, seek);

        self.render_transport(id, ctx);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Column {
    title: Row,
    artist: Row,
    scrubber: Row,
    height: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::Music,
        store::Entry,
        ui::card::{
            resolve_hit,
            tests::{card, entry, measure, paint},
        },
    };
    use crownshell::TextContext;

    fn now_playing(playing: bool, percent: u8) -> Entry {
        entry(
            Notification::builder("Spotify")
                .id(41)
                .content(Content::Music(Music {
                    title: "Blue Monday".into(),
                    artist: "New Order".into(),
                    album_art: None,
                    playing,
                    percent,
                }))
                .build(),
        )
    }

    #[test]
    fn the_card_renders_a_track_it_is_only_observing() {
        // No local player, no album art, nothing but what the phone said.
        let view = MusicView::new(&now_playing(true, 42).notification);
        assert_eq!(view.title.text(), "Blue Monday");
        assert_eq!(view.artist.text(), "New Order");
        assert!(view.playing);
        assert_eq!(view.percent, 42);
        assert!(view.art.is_none());
    }

    /// Spotify posts plain notifications; the profile routes them here, so the
    /// summary and body have to stand in for a payload that never arrives.
    #[test]
    fn a_music_card_without_a_payload_falls_back_to_the_summary_and_body() {
        let view = MusicView::new(
            &Notification::builder("Spotify")
                .summary("Blue Monday")
                .body("New Order")
                .build(),
        );
        assert_eq!(view.title.text(), "Blue Monday");
        assert_eq!(view.artist.text(), "New Order");
        assert!(!view.playing, "an observed track defaults to stopped");
    }

    #[test]
    fn the_toggle_shows_pause_while_playing_and_play_while_stopped() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();

        for (playing, expected) in [(true, MediaAction::Pause), (false, MediaAction::Play)] {
            let mut card = card(&now_playing(playing, 30));
            let hits = paint(&mut card, &mut tcx, &theme);
            let bounds = Rect::new(
                12.0,
                20.0,
                12.0 + theme.metrics.card_width,
                20.0 + card.height(),
            );
            let toggle = Point::new(
                bounds.center().x,
                bounds.y1 - theme.metrics.pad_y - theme.metrics.button / 2.0,
            );
            assert_eq!(
                hits.hit(toggle).map(|hit| &hit.action),
                Some(&CardAction::Media(expected)),
                "the toggle did not match playing = {playing}"
            );
        }
    }

    #[test]
    fn previous_and_next_flank_the_toggle_and_stay_inside_the_card() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&now_playing(true, 30));
        let hits = paint(&mut card, &mut tcx, &theme);
        let bounds = Rect::new(
            12.0,
            20.0,
            12.0 + theme.metrics.card_width,
            20.0 + card.height(),
        );
        let middle = bounds.y1 - theme.metrics.pad_y - theme.metrics.button / 2.0;
        let step = theme.metrics.button + CONTROL_SPACING * 2.0;

        assert_eq!(
            hits.hit(Point::new(bounds.center().x - step, middle))
                .map(|hit| &hit.action),
            Some(&CardAction::Media(MediaAction::Previous))
        );
        assert_eq!(
            hits.hit(Point::new(bounds.center().x + step, middle))
                .map(|hit| &hit.action),
            Some(&CardAction::Media(MediaAction::Next))
        );
        for rect in hits.rects() {
            assert!(
                bounds.union(rect) == bounds,
                "{rect:?} escaped the card bounds {bounds:?}"
            );
        }
    }

    #[test]
    fn a_click_on_the_scrubber_seeks_to_where_it_landed() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&now_playing(true, 10));
        let hits = paint(&mut card, &mut tcx, &theme);

        let band = Rect::new(
            12.0 + theme.metrics.pad_x + theme.metrics.icon * ART_SCALE + theme.metrics.icon_gap,
            0.0,
            12.0 + theme.metrics.card_width - theme.metrics.pad_x,
            0.0,
        );
        let probe = Point::new(band.center().x, 0.0);

        // Find the scrubber by probing the horizontal middle of the band at
        // every y inside the card; the transport row is the only other thing
        // there and it does not reach the band's x.
        let region = (0..400)
            .map(|y| Point::new(probe.x, 20.0 + f64::from(y) * 0.5))
            .filter_map(|point| hits.hit(point))
            .find(|hit| matches!(hit.action, CardAction::Media(MediaAction::Seek(_))))
            .expect("the scrubber pushed no region");

        assert_eq!(
            resolve_hit(region, Point::new(region.rect.x0, region.rect.center().y)),
            CardAction::Media(MediaAction::Seek(0))
        );
        assert_eq!(
            resolve_hit(region, Point::new(region.rect.x1, region.rect.center().y)),
            CardAction::Media(MediaAction::Seek(100))
        );
        assert_eq!(
            resolve_hit(region, region.rect.center()),
            CardAction::Media(MediaAction::Seek(50))
        );
    }

    /// A track advancing is the most common replacement in the whole daemon; if
    /// it re-measured, every music card would re-lay out its text once a second.
    #[test]
    fn advancing_the_play_head_does_not_re_measure_the_card() {
        let theme = Theme::light();
        let mut tcx = TextContext::new();
        let mut card = card(&now_playing(true, 10));
        let height = measure(&mut card, &mut tcx, &theme);

        for percent in [11_u8, 12, 50, 99] {
            card.sync(&now_playing(true, percent), &theme);
            assert!(
                !card.needs_measure(),
                "a seek to {percent}% invalidated the height"
            );
        }
        card.sync(&now_playing(false, 99), &theme);
        assert!(!card.needs_measure(), "pausing invalidated the height");
        assert_eq!(measure(&mut card, &mut tcx, &theme), height);

        card.sync(
            &entry(
                Notification::builder("Spotify")
                    .id(41)
                    .content(Content::Music(Music {
                        title: "Temptation".into(),
                        artist: "New Order".into(),
                        album_art: None,
                        playing: true,
                        percent: 0,
                    }))
                    .build(),
            ),
            &theme,
        );
        assert!(card.needs_measure(), "a new track did not invalidate");
    }
}
