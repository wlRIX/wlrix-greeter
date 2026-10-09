// SPDX-License-Identifier: GPL-3.0-or-later
//! The user pictures in the list.
//!
//! Two kinds, and IRIX showed their state two different ways -- both measured off the real login
//! screen rather than guessed, and both now [`wlrix_ui::icon_state`]'s, which the desktop's icons
//! follow too.
//!
//! **A user with a picture of their own** shows it exactly as it is, in a frame: one black pixel
//! around it at rest; under the pointer, two white ones -- that same pixel and the picture's own
//! outermost row; selected, the same two in lamp yellow. The picture is never tinted.
//!
//! **Everyone else** shows the default picture, IRIX's figure at a workstation, and there **only
//! the white body changes.** At rest it is a 50% checkerboard of white and the palette's rest
//! gray; under the pointer it is solid white; selected, solid yellow. The black outline and the
//! gray shadow under the figure are the same in all three.
//!
//! # Where the pictures come from
//!
//! A user's own: the first of [`User::picture_candidates`] that decodes -- `~/.face.icon`,
//! `~/.face`, then AccountsService's copy, the places `wlrix-lock` looks. Center-cropped to a
//! square, the way the lock crops it to a circle. Read the first time the user is drawn and
//! remembered, including when there is none.
//!
//! The default: `wlrix-greeter/default-user.png` on the XDG data directories, which
//! `just install` puts under the prefix -- the way `wlrix-compositor` finds its minimized-window
//! pictures. When the greeter runs from its own source tree (`just render`, `just preview`), the
//! checkout's `assets/` is the fallback, so a development build shows the picture without an
//! install. With neither, a user without a picture shows their name alone: nothing about logging
//! in depends on a picture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use image::RgbaImage;
use image::imageops::FilterType;
use wlrix_ui::canvas::{Canvas, Rect};
use wlrix_ui::palette::Palette;
use wlrix_ui::{IconState, Rgb};

use crate::users::User;

/// The drawn size of a user picture, in logical pixels: the default artwork's own, and IRIX's.
pub const ICON_SIZE: i32 = 96;

/// The default picture's file name, under the data directory below.
const DEFAULT_PICTURE: &str = "default-user.png";
/// This program's directory under each XDG data directory.
const DATA_SUBDIR: &str = "wlrix-greeter";

/// The largest picture decoded, per axis. Account pictures are a few hundred pixels; a header
/// claiming more than this is refused before anything is allocated. `wlrix-lock`'s limit.
const MAX_DIMENSION: u32 = 4096;

/// The decoded pictures.
pub struct UserIcons {
    /// The default picture at [`ICON_SIZE`], or `None` if it could not be found.
    default: Option<RgbaImage>,
    /// Each user's own picture at [`ICON_SIZE`], by login name, once looked for. `None` is a
    /// user who has none, remembered so their home is not searched again every frame.
    pictures: HashMap<String, Option<RgbaImage>>,
}

impl UserIcons {
    /// Find and decode the default picture. Users' own are read as they are first drawn.
    pub fn new() -> Self {
        let default = find_default().and_then(|path| match decode(&path) {
            Ok(picture) => Some(fit(&picture)),
            Err(why) => {
                eprintln!("wlrix-greeter: {}", why.unwrap_or_default());
                None
            }
        });
        if default.is_none() {
            eprintln!(
                "wlrix-greeter: no usable {DATA_SUBDIR}/{DEFAULT_PICTURE} on the data directories; \
                 users without a picture show their name only"
            );
        }
        Self {
            default,
            pictures: HashMap::new(),
        }
    }

    /// Draw `user`'s picture filling `rect` (expected to be [`ICON_SIZE`] square), in state
    /// `tint`. Nothing at all when there is no picture to draw.
    pub fn draw(
        &mut self,
        canvas: &mut Canvas,
        palette: &Palette,
        rect: Rect,
        user: &User,
        tint: IconState,
    ) {
        let own = self
            .pictures
            .entry(user.name.clone())
            .or_insert_with(|| own_picture(user));
        if let Some(picture) = own {
            draw_own(canvas, palette, rect, picture, tint);
        } else if let Some(default) = &self.default {
            draw_default(canvas, palette, rect, default, tint);
        }
    }
}

impl Default for UserIcons {
    fn default() -> Self {
        Self::new()
    }
}

/// A user's own picture, as it is, in its frame.
fn draw_own(
    canvas: &mut Canvas,
    palette: &Palette,
    rect: Rect,
    picture: &RgbaImage,
    tint: IconState,
) {
    blit(canvas, rect, picture, |_, _, pixel| pixel);
    let color = tint.frame(palette);
    for ring in 0..tint.frame_width() {
        canvas.stroke_rect(rect.inset(ring - 1), color);
    }
}

/// The default picture, its white body in state `tint`'s color.
fn draw_default(
    canvas: &mut Canvas,
    palette: &Palette,
    rect: Rect,
    picture: &RgbaImage,
    tint: IconState,
) {
    blit(canvas, rect, picture, |x, y, pixel| {
        tint.paint(palette, x, y, pixel)
    });
}

/// Composite `picture` centered in `rect`, passing each pixel through `paint` first.
fn blit(
    canvas: &mut Canvas,
    rect: Rect,
    picture: &RgbaImage,
    paint: impl Fn(i32, i32, [u8; 4]) -> [u8; 4],
) {
    let (width, height) = (picture.width() as i32, picture.height() as i32);
    let (left, top) = (
        rect.x + (rect.w - width) / 2,
        rect.y + (rect.h - height) / 2,
    );
    for (x, y, pixel) in picture.enumerate_pixels() {
        let (x, y) = (x as i32, y as i32);
        let [r, g, b, a] = paint(x, y, pixel.0);
        if a != 0 {
            canvas.blend(left + x, top + y, Rgb::from_channels(r, g, b), a);
        }
    }
}

/// The first of `user`'s picture candidates that decodes, fitted to [`ICON_SIZE`].
fn own_picture(user: &User) -> Option<RgbaImage> {
    user.picture_candidates()
        .iter()
        .find_map(|path| match decode(path) {
            Ok(picture) => {
                eprintln!(
                    "wlrix-greeter: picture for {}: {}",
                    user.name,
                    path.display()
                );
                Some(fit(&picture))
            }
            Err(None) => None,
            Err(Some(why)) => {
                eprintln!("wlrix-greeter: {why}");
                None
            }
        })
}

/// Decode one candidate. `Err(None)` is a file that is not there -- or not readable by the
/// greeter's account, which is the same thing from here -- and the ordinary case for most
/// candidates, so not worth a log line.
///
/// Regular files only. `is_file` follows a symlink and refuses what is at the end of it unless it
/// is a plain file, so a `~/.face` pointing at a FIFO or a device cannot leave the login screen
/// blocked on a read that never finishes.
fn decode(path: &Path) -> Result<RgbaImage, Option<String>> {
    if !path.is_file() {
        return Err(None);
    }
    let reader = match image::ImageReader::open(path) {
        Ok(reader) => reader,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return Err(None),
        Err(err) => return Err(Some(format!("could not open {}: {err}", path.display()))),
    };
    let mut reader = reader
        .with_guessed_format()
        .map_err(|err| Some(format!("could not read {}: {err}", path.display())))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    reader.limits(limits);
    reader
        .decode()
        .map(|picture| picture.to_rgba8())
        .map_err(|err| Some(format!("could not decode {}: {err}", path.display())))
}

/// `source` center-cropped to a square and scaled to [`ICON_SIZE`].
///
/// A picture already that size is returned untouched, so the default artwork's pixels -- which
/// the recoloring matches exactly -- are never resampled.
fn fit(source: &RgbaImage) -> RgbaImage {
    let size = ICON_SIZE as u32;
    let (w, h) = source.dimensions();
    if (w, h) == (size, size) {
        return source.clone();
    }
    let side = w.min(h).max(1);
    let cropped = image::imageops::crop_imm(source, (w - side) / 2, (h - side) / 2, side, side);
    image::imageops::resize(&*cropped, size, size, FilterType::Lanczos3)
}

/// The default picture: installed first, then the source tree's.
fn find_default() -> Option<PathBuf> {
    data_dirs()
        .into_iter()
        .map(|dir| dir.join(DATA_SUBDIR).join(DEFAULT_PICTURE))
        .chain(std::iter::once(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join(DEFAULT_PICTURE),
        ))
        .find(|path| path.is_file())
}

/// `$XDG_DATA_HOME`, then `$XDG_DATA_DIRS`, with the spec's defaults for either.
fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")));
    dirs.extend(home);
    let system = std::env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(std::env::split_paths(&system));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> &'static Palette {
        wlrix_ui::palette::resolve(None).0
    }

    fn opaque(rgb: Rgb) -> [u8; 4] {
        let (r, g, b) = rgb.channels();
        [r, g, b, 255]
    }

    /// The default picture, decoded the way the greeter decodes it.
    fn default_picture() -> RgbaImage {
        let path = find_default().expect("the checkout carries assets/default-user.png");
        fit(&decode(&path).expect("decodes"))
    }

    /// A scratch directory for one test.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wlrix-greeter-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

    fn user(name: &str, home: Option<&Path>) -> User {
        User {
            name: name.into(),
            full_name: None,
            uid: 1000,
            home: home.map(Path::to_path_buf),
        }
    }

    #[test]
    fn the_default_picture_is_found_at_its_own_size() {
        // The fallback a development build relies on, and a check that the artwork decodes --
        // it is 8-bit colormap, the kind of PNG that used to fail elsewhere in wlRIX.
        assert_eq!(default_picture().dimensions(), (96, 96));
    }

    #[test]
    fn only_the_default_pictures_white_body_changes() {
        // Measured off IRIX: the outline and the shadow are identical in all three states.
        let palette = palette();
        let picture = default_picture();
        let has = |rgb: [u8; 4]| picture.pixels().any(|px| px.0 == rgb);
        let (black, shadow) = ([0, 0, 0, 255], [0x86, 0x86, 0x86, 255]);
        assert!(
            has(black) && has(shadow) && has([255; 4]),
            "the artwork's three colors"
        );
        for tint in [IconState::Normal, IconState::Hover, IconState::Selected] {
            for rgb in [black, shadow] {
                assert_eq!(tint.paint(palette, 0, 0, rgb), rgb, "{tint:?}");
                assert_eq!(tint.paint(palette, 1, 0, rgb), rgb, "{tint:?}");
            }
        }
        assert_eq!(
            IconState::Selected.paint(palette, 0, 0, [255; 4]),
            opaque(palette.icon_tint_selected)
        );
        assert_eq!(
            IconState::Hover.paint(palette, 0, 0, [255; 4]),
            opaque(palette.icon_tint_hover)
        );
    }

    #[test]
    fn an_own_picture_is_framed_black_and_then_white_or_yellow_two_deep() {
        // A 96px picture of solid red, drawn into a canvas of the list's gray.
        let palette = palette();
        let red = Rgb::from_channels(0xc0, 0x20, 0x20);
        let picture = RgbaImage::from_pixel(96, 96, image::Rgba(opaque(red)));
        let rect = Rect::new(10, 10, ICON_SIZE, ICON_SIZE);
        let at = |tint: IconState, x: i32, y: i32| {
            let mut pixels = vec![0u8; 120 * 120 * 4];
            let mut canvas = Canvas::new(&mut pixels, 120, 120);
            canvas.fill_rect(Rect::new(0, 0, 120, 120), palette.list_background);
            draw_own(&mut canvas, palette, rect, &picture, tint);
            let i = ((y * 120 + x) * 4) as usize;
            // Argb8888 in memory is B, G, R, A.
            Rgb::from_channels(pixels[i + 2], pixels[i + 1], pixels[i])
        };
        let same = |a: Rgb, b: Rgb| a.channels() == b.channels();

        // At rest: the pixel just outside is black, and the picture is untouched to its edge.
        assert!(same(at(IconState::Normal, 9, 50), palette.outer_line));
        assert!(same(at(IconState::Normal, 10, 50), red));
        assert!(same(at(IconState::Normal, 8, 50), palette.list_background));

        // Highlighted: that pixel and the picture's outermost row, and no further in.
        for (tint, color) in [
            (IconState::Hover, palette.icon_tint_hover),
            (IconState::Selected, palette.icon_tint_selected),
        ] {
            assert!(same(at(tint, 9, 50), color), "{tint:?}");
            assert!(same(at(tint, 10, 50), color), "{tint:?}");
            assert!(same(at(tint, 11, 50), red), "{tint:?}");
            assert!(same(at(tint, 105, 50), color), "{tint:?} right edge");
            assert!(same(at(tint, 106, 50), color), "{tint:?} right outside");
            assert!(same(at(tint, 50, 106), color), "{tint:?} bottom outside");
        }
    }

    #[test]
    fn an_own_picture_is_cropped_square_from_the_middle() {
        let home = scratch("own");
        // Wider than tall: a 200x100 PNG, left quarter blue, the rest green.
        let mut source = RgbaImage::from_pixel(200, 100, image::Rgba([0, 200, 0, 255]));
        for y in 0..100 {
            for x in 0..50 {
                source.put_pixel(x, y, image::Rgba([0, 0, 200, 255]));
            }
        }
        source
            .save_with_format(home.join(".face"), image::ImageFormat::Png)
            .expect("write the picture");

        let picture = own_picture(&user("someone", Some(&home))).expect("found");
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(picture.dimensions(), (96, 96));
        // The middle square kept, so the blue quarter is cropped away entirely.
        assert!(picture.pixels().all(|px| px.0[1] > px.0[2]), "no blue left");
    }

    #[test]
    fn face_icon_comes_before_face() {
        let home = scratch("order");
        RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]))
            .save_with_format(home.join(".face.icon"), image::ImageFormat::Png)
            .unwrap();
        RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 255, 255]))
            .save_with_format(home.join(".face"), image::ImageFormat::Png)
            .unwrap();
        let picture = own_picture(&user("someone", Some(&home))).expect("found");
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(picture.get_pixel(48, 48).0[0], 255, "the .face.icon one");
    }

    #[test]
    fn nothing_but_a_regular_file_is_read() {
        // A directory where the picture would be: refused without trying to read it.
        let home = scratch("notafile");
        std::fs::create_dir(home.join(".face")).unwrap();
        assert!(matches!(decode(&home.join(".face")), Err(None)));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_candidates_are_the_locks() {
        let paths = user("vic", Some(Path::new("/home/vic"))).picture_candidates();
        assert_eq!(
            paths,
            [
                PathBuf::from("/home/vic/.face.icon"),
                PathBuf::from("/home/vic/.face"),
                PathBuf::from("/var/lib/AccountsService/icons/vic"),
            ]
        );
        // No home: AccountsService alone. A name that would climb out of its directory: nothing.
        assert_eq!(user("vic", None).picture_candidates().len(), 1);
        assert!(user("..", None).picture_candidates().is_empty());
    }
}
