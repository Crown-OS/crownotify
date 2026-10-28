//! Icons, and the decoder for the raw image hint `Notify` can carry.

use std::{path::PathBuf, sync::Arc};

use crate::error::{Error, Result};

/// Where a picture comes from.
///
/// Resolution is deliberately deferred: a notification is created on a
/// transport thread, but themes, files and decoders are the icon loader's
/// business on the UI thread, so the model only records what was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Icon {
    /// An icon-theme name, resolved through the XDG icon theme.
    Named(String),
    /// An absolute path to an image file.
    Path(PathBuf),
    /// Pixels the sender handed us directly. `Arc` because the same image is
    /// shared between the toast and the center without being copied.
    Data(Arc<ImageData>),
}

/// Straight-alpha RGBA8, row-major, no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Largest edge accepted from a hint. A notification icon is drawn at 38 px;
/// anything beyond this is either a mistake or an attempt to make the daemon
/// allocate gigabytes on behalf of an unprivileged sender.
const MAX_DIMENSION: i32 = 8192;

impl ImageData {
    /// Decodes the freedesktop `image-data` hint (iiibiiay).
    ///
    /// The wire format is a GdkPixbuf dump, which means three things worth
    /// spelling out, all of which are load-bearing:
    ///
    /// * rows are padded to `rowstride`, which is *not* `width * channels` in
    ///   general, so the padding has to be stepped over rather than read;
    /// * the pixels may be 3-channel RGB, which is expanded to opaque RGBA
    ///   here so that every consumer sees one layout;
    /// * the alpha is straight, not premultiplied, which is what the renderer
    ///   wants, so it is passed through as it is.
    ///
    /// Every field comes from another process. Geometry that does not describe
    /// the buffer it arrived with is reported as [`Error::IconDecode`]; nothing
    /// in here indexes past the end of `data` or panics.
    pub fn from_hint(
        width: i32,
        height: i32,
        rowstride: i32,
        has_alpha: bool,
        bits_per_sample: i32,
        channels: i32,
        data: Vec<u8>,
    ) -> Result<Self> {
        if bits_per_sample != 8 {
            return Err(Error::IconDecode(format!(
                "bits_per_sample is {bits_per_sample}, only 8 is defined"
            )));
        }
        if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
            return Err(Error::IconDecode(format!(
                "implausible geometry {width}x{height}"
            )));
        }
        if channels != 3 && channels != 4 {
            return Err(Error::IconDecode(format!(
                "{channels} channels, expected 3 (RGB) or 4 (RGBA)"
            )));
        }

        let width = width as usize;
        let height = height as usize;
        let channels = channels as usize;
        let packed = width * channels;
        // A sender that leaves rowstride unset means "tightly packed".
        let stride = if rowstride <= 0 {
            packed
        } else {
            rowstride as usize
        };
        if stride < packed {
            return Err(Error::IconDecode(format!(
                "rowstride {stride} is shorter than a {width}px row of {channels} channels"
            )));
        }

        let needed = stride
            .checked_mul(height - 1)
            .and_then(|rows| rows.checked_add(packed))
            .ok_or_else(|| Error::IconDecode("geometry overflows the address space".to_string()))?;
        if data.len() < needed {
            return Err(Error::IconDecode(format!(
                "buffer holds {} bytes, {width}x{height} at stride {stride} needs {needed}",
                data.len()
            )));
        }

        // Three channels have no alpha to read, and `has_alpha == false` with
        // four means the fourth byte is padding rather than coverage.
        let opaque = !has_alpha || channels == 3;

        if channels == 4 && stride == packed {
            let mut rgba = data;
            rgba.truncate(packed * height);
            if opaque {
                rgba.chunks_exact_mut(4)
                    .for_each(|pixel| pixel[3] = u8::MAX);
            }
            return Ok(Self {
                width: width as u32,
                height: height as u32,
                rgba,
            });
        }

        let mut rgba = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            let start = row * stride;
            for source in data[start..start + packed].chunks_exact(channels) {
                let alpha = if opaque { u8::MAX } else { source[3] };
                rgba.extend_from_slice(&[source[0], source[1], source[2], alpha]);
            }
        }

        Ok(Self {
            width: width as u32,
            height: height as u32,
            rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_pixels_survive_a_tightly_packed_decode() {
        let data = vec![10, 20, 30, 255, 40, 50, 60, 255];
        let image = ImageData::from_hint(2, 1, 8, true, 8, 4, data).expect("should decode");

        assert_eq!(image.width, 2);
        assert_eq!(image.height, 1);
        assert_eq!(image.rgba, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn three_channel_rgb_is_expanded_to_opaque_rgba() {
        let data = vec![1, 2, 3, 4, 5, 6];
        let image = ImageData::from_hint(2, 1, 6, false, 8, 3, data).expect("should decode");

        assert_eq!(
            image.rgba,
            vec![1, 2, 3, 255, 4, 5, 6, 255],
            "RGB should gain a fully opaque alpha channel"
        );
    }

    #[test]
    fn row_padding_is_stepped_over_rather_than_copied() {
        // 1 px wide, 2 rows, stride 8: 4 bytes of pixel and 4 of padding.
        let data = vec![1, 2, 3, 255, 99, 99, 99, 99, 4, 5, 6, 255, 99, 99, 99, 99];
        let image = ImageData::from_hint(1, 2, 8, true, 8, 4, data).expect("should decode");

        assert_eq!(
            image.rgba,
            vec![1, 2, 3, 255, 4, 5, 6, 255],
            "the padding bytes must not reach the output"
        );
    }

    #[test]
    fn rgb_row_padding_is_stepped_over_too() {
        let data = vec![1, 2, 3, 0, 4, 5, 6, 0];
        let image = ImageData::from_hint(1, 2, 4, false, 8, 3, data).expect("should decode");

        assert_eq!(image.rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn straight_alpha_reaches_the_renderer_unscaled() {
        let image =
            ImageData::from_hint(1, 1, 4, true, 8, 4, vec![255, 128, 0, 128]).expect("decodes");

        assert_eq!(
            image.rgba,
            vec![255, 128, 0, 128],
            "colour channels must not be scaled by the alpha"
        );
    }

    #[test]
    fn four_channels_without_has_alpha_are_forced_opaque() {
        let image =
            ImageData::from_hint(1, 1, 4, false, 8, 4, vec![10, 20, 30, 7]).expect("decodes");

        assert_eq!(
            image.rgba,
            vec![10, 20, 30, 255],
            "the fourth byte is padding when has_alpha is false"
        );
    }

    #[test]
    fn a_missing_rowstride_is_treated_as_tightly_packed() {
        let image = ImageData::from_hint(2, 1, 0, true, 8, 4, vec![1, 2, 3, 255, 4, 5, 6, 255])
            .expect("decodes");
        assert_eq!(image.rgba.len(), 8, "got {} bytes", image.rgba.len());
    }

    #[test]
    fn a_non_eight_bit_sample_depth_is_rejected() {
        for depth in [1, 4, 16, 32] {
            let error = ImageData::from_hint(1, 1, 4, true, depth, 4, vec![0; 4])
                .expect_err("only 8 bits per sample is defined");
            assert!(
                matches!(error, Error::IconDecode(_)),
                "expected IconDecode for depth {depth}, got {error:?}"
            );
        }
    }

    #[test]
    fn a_buffer_too_short_for_the_stated_geometry_is_rejected() {
        let error = ImageData::from_hint(64, 64, 256, true, 8, 4, vec![0; 16])
            .expect_err("16 bytes cannot hold a 64x64 image");
        assert!(matches!(error, Error::IconDecode(_)), "got {error:?}");
    }

    #[test]
    fn a_rowstride_narrower_than_a_row_is_rejected() {
        let error = ImageData::from_hint(4, 2, 8, true, 8, 4, vec![0; 64])
            .expect_err("stride 8 cannot hold a 4px RGBA row");
        assert!(matches!(error, Error::IconDecode(_)), "got {error:?}");
    }

    #[test]
    fn implausible_geometry_is_rejected_before_anything_is_allocated() {
        for (width, height) in [(0, 1), (1, 0), (-4, 4), (4, -4), (100_000, 1)] {
            let error = ImageData::from_hint(width, height, 0, true, 8, 4, vec![0; 4])
                .expect_err("out-of-range geometry should be rejected");
            assert!(
                matches!(error, Error::IconDecode(_)),
                "expected IconDecode for {width}x{height}, got {error:?}"
            );
        }
    }

    #[test]
    fn an_unsupported_channel_count_is_rejected() {
        for channels in [0, 1, 2, 5] {
            let error = ImageData::from_hint(1, 1, 0, true, 8, channels, vec![0; 8])
                .expect_err("only 3 and 4 channels are defined");
            assert!(
                matches!(error, Error::IconDecode(_)),
                "expected IconDecode for {channels} channels, got {error:?}"
            );
        }
    }

    #[test]
    fn icons_compare_by_value_so_a_replacement_can_be_diffed() {
        assert_eq!(Icon::Named("mail".into()), Icon::Named("mail".into()));
        assert_ne!(
            Icon::Named("mail".into()),
            Icon::Path(PathBuf::from("/usr/share/icons/mail.png"))
        );

        let image = Arc::new(ImageData {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 4],
        });
        assert_eq!(Icon::Data(image.clone()), Icon::Data(image));
    }
}
