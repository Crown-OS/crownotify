//! Turns an [`Icon`](crate::model::Icon) into pixels Vello can draw, with a
//! cache that survives across frames.
//!
//! `paint` runs at up to 144 Hz, so nothing here may touch a filesystem or a
//! decoder on the common path: every result — including a failed one — is
//! cached by `(icon identity, size)`, and [`IconTheme`] scans directories
//! once rather than per call. [`IconLoader::sweep`] is how an icon that
//! stops being drawn (the notification it belonged to closed) eventually
//! leaves the cache; nothing evicts eagerly, because eviction on every miss
//! would defeat the point of caching a 144 Hz-hot path.
//!
//! ## Premultiplication
//!
//! Vello's [`peniko::ImageData`] with [`ImageFormat::Rgba8`] and
//! [`ImageAlphaType::AlphaPremultiplied`] expects every colour channel
//! already scaled by that pixel's alpha. The three sources this loader reads
//! disagree on that by default, so each decode path says explicitly, at the
//! point it matters, which way it had to convert:
//!
//! * `image` (PNG/JPEG/WebP/BMP) hands back straight alpha — the formats
//!   themselves store colour and coverage independently — so it is
//!   premultiplied here after decoding.
//! * `resvg`/`tiny-skia` rasterise directly into premultiplied RGBA
//!   (`tiny_skia::Pixmap::data` is documented as premultiplied, byte order
//!   RGBA), so its output is used unchanged.
//! * [`ImageData`](crate::model::ImageData) (the freedesktop `image-data`
//!   hint) is premultiplied by [`ImageData::from_hint`] before it ever
//!   reaches this module, per its own doc comment — also used unchanged,
//!   except when it must be resized, which needs a straight-alpha round
//!   trip in between (see `decode_data` below).

pub mod theme;

use std::{collections::HashMap, fs, path::Path, sync::Arc};

use vello::peniko::{Blob, ImageAlphaType, ImageData as PenikoImage, ImageFormat};

pub use theme::IconTheme;

use crate::model::icon::{Icon, ImageData};

/// Identifies one cached decode: which icon, at which pixel size.
///
/// [`Icon::Data`] is keyed by the address of its shared pixel buffer rather
/// than its contents, which is the only way to make this key cheap to hash
/// — hashing or comparing the pixels themselves would cost as much as the
/// decode this cache exists to avoid. The tradeoff is that the key is only
/// valid for as long as that `Arc` allocation lives; once every clone of it
/// is dropped, the allocator is free to reuse the address for something
/// else, which could in principle serve a stale image for one `sweep`
/// interval. `sweep` is expected to run at least once between a
/// notification closing and a new, unrelated one reusing that address in
/// practice, which is the same assumption GPU texture caches keyed on
/// pointer identity make.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CacheKey {
    Named(String, u32),
    Path(std::path::PathBuf, u32),
    Data(usize, u32),
}

impl CacheKey {
    fn for_icon(icon: &Icon, size: u32) -> Self {
        match icon {
            Icon::Named(name) => Self::Named(name.clone(), size),
            Icon::Path(path) => Self::Path(path.clone(), size),
            Icon::Data(data) => Self::Data(Arc::as_ptr(data) as usize, size),
        }
    }
}

struct CacheEntry {
    image: Option<PenikoImage>,
    touched: bool,
}

/// Decodes and rasterises icons on demand, caching every result — including
/// failures — keyed by icon identity and requested size.
pub struct IconLoader {
    theme: IconTheme,
    cache: HashMap<CacheKey, CacheEntry>,
    #[cfg(test)]
    decodes: u32,
}

impl IconLoader {
    pub fn new() -> Self {
        Self {
            theme: IconTheme::detect(),
            cache: HashMap::new(),
            #[cfg(test)]
            decodes: 0,
        }
    }

    /// Builds a loader against an already-resolved theme, so callers (and
    /// tests) that need a specific theme instead of whatever `detect` finds
    /// don't have to touch the environment to get one.
    #[cfg(test)]
    fn with_theme(theme: IconTheme) -> Self {
        Self {
            theme,
            cache: HashMap::new(),
            decodes: 0,
        }
    }

    /// Rasterised RGBA at `size` logical px, ready to blit. `None` when the
    /// icon cannot be resolved or decoded — callers draw a fallback glyph.
    ///
    /// A cache hit is a hashmap lookup plus an `Arc` clone of the pixel
    /// blob, never a re-decode, whether the icon resolved or not.
    pub fn load(&mut self, icon: &Icon, size: u32) -> Option<PenikoImage> {
        let key = CacheKey::for_icon(icon, size);
        if let Some(entry) = self.cache.get_mut(&key) {
            entry.touched = true;
            return entry.image.clone();
        }

        let image = self.decode(icon, size);
        self.cache.insert(
            key,
            CacheEntry {
                image: image.clone(),
                touched: true,
            },
        );
        image
    }

    /// Drops cached entries not touched since the last sweep.
    pub fn sweep(&mut self) {
        self.cache.retain(|_, entry| entry.touched);
        for entry in self.cache.values_mut() {
            entry.touched = false;
        }
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    fn decode(&mut self, icon: &Icon, size: u32) -> Option<PenikoImage> {
        #[cfg(test)]
        {
            self.decodes += 1;
        }
        match icon {
            Icon::Named(name) => {
                let path = self.theme.lookup(name, size)?;
                decode_path(&path, size)
            }
            Icon::Path(path) => decode_path(path, size),
            Icon::Data(data) => decode_data(data, size),
        }
    }
}

impl Default for IconLoader {
    fn default() -> Self {
        Self::new()
    }
}

fn decode_path(path: &Path, size: u32) -> Option<PenikoImage> {
    let is_svg = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"));
    let bytes = fs::read(path).ok()?;
    if is_svg {
        decode_svg(&bytes, size)
    } else {
        decode_raster(&bytes, size)
    }
}

/// Longest-edge-fit dimensions for downscaling `width`x`height` to `size`.
/// Never upscales — an icon smaller than requested is drawn at its native
/// size rather than blown up and blurred.
fn scaled_dimensions(width: u32, height: u32, size: u32) -> (u32, u32) {
    let longest = width.max(height);
    if size == 0 || width == 0 || height == 0 || longest <= size {
        return (width, height);
    }
    let scale = f64::from(size) / f64::from(longest);
    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}

/// Scales every pixel's colour channels by its own alpha, in place.
///
/// Matches the `+ 127` rounding [`crate::model::icon`] uses for the same
/// operation, so a pixel that survives both a hint decode and a raster
/// decode round-trips identically.
fn premultiply(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
}

/// Reverses [`premultiply`], needed only when a source that already arrived
/// premultiplied ([`Icon::Data`]) has to be resized: `image::imageops::resize`
/// blends neighbouring pixels assuming straight alpha, and blending
/// premultiplied colour directly would darken translucent edges.
fn unpremultiply(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = pixel[3];
        if alpha == 0 {
            // Colour is unobservable at zero coverage; leave it as-is rather
            // than divide by zero.
            continue;
        }
        for channel in &mut pixel[..3] {
            let scaled = (u16::from(*channel) * 255 + u16::from(alpha) / 2) / u16::from(alpha);
            *channel = scaled.min(255) as u8;
        }
    }
}

fn decode_raster(bytes: &[u8], size: u32) -> Option<PenikoImage> {
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (src_width, src_height) = image.dimensions();
    let (width, height) = scaled_dimensions(src_width, src_height, size);
    let image = if (width, height) != (src_width, src_height) {
        image::imageops::resize(&image, width, height, image::imageops::FilterType::Triangle)
    } else {
        image
    };

    let mut rgba = image.into_raw();
    // `image`'s PNG/JPEG/WebP/BMP decoders all hand back straight alpha —
    // none of those formats store colour pre-scaled by coverage — so Vello's
    // premultiplied expectation is met by converting here, once, at decode
    // time rather than on every frame this icon is drawn.
    premultiply(&mut rgba);

    Some(PenikoImage {
        data: Blob::from(rgba),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width,
        height,
    })
}

fn decode_svg(bytes: &[u8], size: u32) -> Option<PenikoImage> {
    let edge = size.max(1);
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &options).ok()?;

    let source = tree.size();
    let target = edge as f32;
    let scale = (target / source.width()).min(target / source.height());
    let transform = tiny_skia::Transform::from_scale(scale, scale);

    let mut pixmap = tiny_skia::Pixmap::new(edge, edge)?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // `tiny_skia::Pixmap::data` is documented as premultiplied RGBA already
    // — resvg rasterises straight into the format Vello wants, so unlike the
    // raster path above there is nothing to convert here.
    let rgba = pixmap.data().to_vec();

    Some(PenikoImage {
        data: Blob::from(rgba),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width: edge,
        height: edge,
    })
}

/// Wraps a notification's own decoded pixels so they can be handed to
/// [`Blob::new`] without copying them — the loader never mutates this data,
/// only shares it, so the toast and the center can point at the same
/// allocation the D-Bus thread produced.
struct SharedRgba(Arc<ImageData>);

impl AsRef<[u8]> for SharedRgba {
    fn as_ref(&self) -> &[u8] {
        &self.0.rgba
    }
}

fn decode_data(data: &Arc<ImageData>, size: u32) -> Option<PenikoImage> {
    let expected = (data.width as usize)
        .checked_mul(data.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))?;
    if data.width == 0 || data.height == 0 || data.rgba.len() != expected {
        // The sender's stated geometry does not describe the buffer it
        // handed over. Treating that as a decode failure (rather than
        // indexing into a buffer that doesn't match it) is what keeps this
        // infallible against a malicious or merely buggy notifier.
        return None;
    }

    let (width, height) = scaled_dimensions(data.width, data.height, size);
    if (width, height) == (data.width, data.height) {
        let blob = Blob::new(Arc::new(SharedRgba(data.clone())));
        return Some(PenikoImage {
            data: blob,
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
            width,
            height,
        });
    }

    // `ImageData` arrives already premultiplied (see its own doc comment),
    // but `image::imageops::resize` blends neighbours assuming straight
    // alpha, so this round-trips through straight alpha for the resize and
    // back rather than resampling premultiplied colour directly.
    let mut straight = data.rgba.clone();
    unpremultiply(&mut straight);
    let buffer = image::RgbaImage::from_raw(data.width, data.height, straight)?;
    let resized = image::imageops::resize(
        &buffer,
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    let mut rgba = resized.into_raw();
    premultiply(&mut rgba);

    Some(PenikoImage {
        data: Blob::from(rgba),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_png(path: &Path, width: u32, height: u32, pixel: [u8; 4]) {
        let mut image = image::RgbaImage::new(width, height);
        for p in image.pixels_mut() {
            *p = image::Rgba(pixel);
        }
        image.save(path).expect("write synthetic png");
    }

    #[test]
    fn a_synthetic_png_round_trips_to_the_requested_dimensions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("icon.png");
        write_png(&path, 64, 64, [255, 0, 0, 255]);

        let mut loader = IconLoader::new();
        let image = loader
            .load(&Icon::Path(path), 32)
            .expect("a valid png should decode");

        assert_eq!(
            image.width, 32,
            "width should be downscaled to the requested size"
        );
        assert_eq!(
            image.height, 32,
            "height should be downscaled to the requested size"
        );
        assert_eq!(image.alpha_type, ImageAlphaType::AlphaPremultiplied);
    }

    #[test]
    fn a_smaller_source_is_not_upscaled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("small.png");
        write_png(&path, 16, 16, [0, 255, 0, 255]);

        let mut loader = IconLoader::new();
        let image = loader
            .load(&Icon::Path(path), 64)
            .expect("a valid png should decode");

        assert_eq!(
            image.width, 16,
            "a 16px source should not be blown up to 64px"
        );
        assert_eq!(image.height, 16);
    }

    #[test]
    fn a_half_transparent_pixel_is_premultiplied_on_the_raster_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("translucent.png");
        // Straight alpha: full-intensity red at 50% coverage.
        write_png(&path, 4, 4, [255, 0, 0, 128]);

        let mut loader = IconLoader::new();
        let image = loader
            .load(&Icon::Path(path), 4)
            .expect("a valid png should decode");

        let pixels = image.data.data();
        assert_eq!(
            pixels[0..4],
            [128, 0, 0, 128],
            "the red channel should be scaled down by the 50% alpha, got {:?}",
            &pixels[0..4]
        );
    }

    #[test]
    fn repeated_loads_of_the_same_icon_and_size_hit_the_cache() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cached.png");
        write_png(&path, 8, 8, [1, 2, 3, 255]);

        let mut loader = IconLoader::new();
        let icon = Icon::Path(path);
        loader.load(&icon, 8).expect("first load should decode");
        loader
            .load(&icon, 8)
            .expect("second load should hit the cache");
        loader
            .load(&icon, 8)
            .expect("third load should hit the cache");

        assert_eq!(
            loader.decodes, 1,
            "only the first load should have decoded anything"
        );
    }

    #[test]
    fn a_different_size_for_the_same_icon_is_a_separate_cache_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("multisize.png");
        write_png(&path, 64, 64, [1, 2, 3, 255]);

        let mut loader = IconLoader::new();
        let icon = Icon::Path(path);
        loader.load(&icon, 16).expect("16px load should decode");
        loader.load(&icon, 32).expect("32px load should decode");

        assert_eq!(loader.decodes, 2, "each distinct size should decode once");
    }

    #[test]
    fn a_missing_file_returns_none_and_is_negatively_cached() {
        let mut loader = IconLoader::new();
        let icon = Icon::Path(PathBuf::from("/nonexistent/does-not-exist.png"));

        assert!(loader.load(&icon, 32).is_none());
        assert!(loader.load(&icon, 32).is_none());

        assert_eq!(
            loader.decodes, 1,
            "the second lookup should have used the negative cache"
        );
    }

    #[test]
    fn an_unresolvable_named_icon_returns_none() {
        let theme = IconTheme::with_name("hicolor-does-not-exist-anywhere");
        let mut loader = IconLoader::with_theme(theme);

        assert!(
            loader
                .load(&Icon::Named("no-such-icon-xyz".into()), 32)
                .is_none()
        );
    }

    #[test]
    fn icon_data_with_geometry_that_does_not_match_its_buffer_does_not_panic() {
        let bogus = Arc::new(ImageData {
            width: 100,
            height: 100,
            rgba: vec![0, 0, 0, 0], // far too short for 100x100
        });

        let mut loader = IconLoader::new();
        let result = loader.load(&Icon::Data(bogus), 32);
        assert!(
            result.is_none(),
            "mismatched geometry should fail decode, not panic"
        );
    }

    #[test]
    fn icon_data_at_its_native_size_is_shared_without_copying() {
        let data = Arc::new(ImageData {
            width: 2,
            height: 2,
            rgba: vec![
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
            ],
        });

        let mut loader = IconLoader::new();
        let image = loader
            .load(&Icon::Data(data.clone()), 2)
            .expect("valid image data should decode");

        assert_eq!(image.width, 2);
        assert_eq!(image.height, 2);
        assert_eq!(image.data.data(), data.rgba.as_slice());
    }

    #[test]
    fn a_large_icon_data_source_is_downscaled_so_the_cache_holds_the_small_version() {
        let width = 256;
        let height = 256;
        let rgba = vec![200u8; (width * height * 4) as usize];
        let data = Arc::new(ImageData {
            width,
            height,
            rgba,
        });

        let mut loader = IconLoader::new();
        let image = loader
            .load(&Icon::Data(data), 32)
            .expect("valid image data should decode");

        assert_eq!(
            image.width, 32,
            "a 256px source requested at 32px should be downscaled"
        );
        assert_eq!(image.height, 32);
    }

    #[test]
    fn sweep_drops_entries_that_were_not_touched_since_the_last_sweep() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("swept.png");
        write_png(&path, 8, 8, [1, 2, 3, 255]);

        let mut loader = IconLoader::new();
        let icon = Icon::Path(path);
        loader.load(&icon, 8).expect("first load should decode");

        loader.sweep(); // first sweep: entry was touched, survives, flag resets
        loader.sweep(); // second sweep: entry was not touched since, evicted

        loader
            .load(&icon, 8)
            .expect("load after eviction should decode again");
        assert_eq!(loader.decodes, 2, "eviction should force a fresh decode");
    }

    #[test]
    fn clear_drops_every_cached_entry_regardless_of_touch_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cleared.png");
        write_png(&path, 8, 8, [1, 2, 3, 255]);

        let mut loader = IconLoader::new();
        let icon = Icon::Path(path);
        loader.load(&icon, 8).expect("first load should decode");
        loader.clear();
        loader
            .load(&icon, 8)
            .expect("load after clear should decode again");

        assert_eq!(loader.decodes, 2);
    }
}
