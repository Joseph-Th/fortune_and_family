//! Indexed pixel buffers and the compositing operations shared by every renderer.

use super::color::TRANSPARENT_INDEX;

/// A rectangular region measured in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A palette-indexed image where index zero is transparent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Canvas {
    /// Creates a fully transparent canvas.
    ///
    /// # Panics
    ///
    /// Panics when either dimension is zero, a dimension cannot be addressed by the signed
    /// coordinate API, or the total pixel count overflows `usize`.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        assert!(
            width > 0 && height > 0,
            "canvas dimensions must be positive"
        );
        assert!(
            i32::try_from(width).is_ok() && i32::try_from(height).is_ok(),
            "canvas dimensions must fit i32 coordinates"
        );
        let count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .expect("canvas pixel count must fit usize");
        Self {
            width,
            height,
            pixels: vec![TRANSPARENT_INDEX; count],
        }
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    #[must_use]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && u32::try_from(x).is_ok_and(|x| x < self.width)
            && u32::try_from(y).is_ok_and(|y| y < self.height)
    }

    fn offset(&self, x: i32, y: i32) -> Option<usize> {
        if !self.contains(x, y) {
            return None;
        }
        let x = usize::try_from(x).ok()?;
        let y = usize::try_from(y).ok()?;
        let width = usize::try_from(self.width).ok()?;
        Some(y * width + x)
    }

    /// Returns the palette index at `(x, y)`, or transparent when out of bounds.
    #[must_use]
    pub fn get(&self, x: i32, y: i32) -> u8 {
        self.offset(x, y)
            .map_or(TRANSPARENT_INDEX, |offset| self.pixels[offset])
    }

    /// Writes a palette index, ignoring writes outside the canvas.
    pub fn set(&mut self, x: i32, y: i32, index: u8) {
        if let Some(offset) = self.offset(x, y) {
            self.pixels[offset] = index;
        }
    }

    pub fn clear(&mut self) {
        self.pixels.fill(TRANSPARENT_INDEX);
    }

    /// Fills the portion of `rect` that intersects this canvas.
    ///
    /// # Panics
    ///
    /// Panics only if this canvas violates the signed-coordinate invariant established by
    /// [`Canvas::new`].
    pub fn fill_rect(&mut self, rect: Rect, index: u8) {
        if rect.is_empty() {
            return;
        }
        let start_x = i64::from(rect.x).max(0);
        let start_y = i64::from(rect.y).max(0);
        let end_x = (i64::from(rect.x) + i64::from(rect.width)).min(i64::from(self.width));
        let end_y = (i64::from(rect.y) + i64::from(rect.height)).min(i64::from(self.height));
        if start_x >= end_x || start_y >= end_y {
            return;
        }
        for y in i32::try_from(start_y).expect("clipped y must fit i32")
            ..i32::try_from(end_y).expect("clipped y must fit i32")
        {
            for x in i32::try_from(start_x).expect("clipped x must fit i32")
                ..i32::try_from(end_x).expect("clipped x must fit i32")
            {
                self.set(x, y, index);
            }
        }
    }

    /// Draws `source` at `(x, y)`, skipping transparent source pixels.
    ///
    /// # Panics
    ///
    /// Panics only if `source` violates the signed-coordinate invariant established by
    /// [`Canvas::new`].
    pub fn blit(&mut self, source: &Self, x: i32, y: i32) {
        for source_y in 0..i32::try_from(source.height).expect("source height must fit i32") {
            for source_x in 0..i32::try_from(source.width).expect("source width must fit i32") {
                let index = source.get(source_x, source_y);
                if index != TRANSPARENT_INDEX {
                    let destination_x = i64::from(x) + i64::from(source_x);
                    let destination_y = i64::from(y) + i64::from(source_y);
                    if let (Ok(destination_x), Ok(destination_y)) =
                        (i32::try_from(destination_x), i32::try_from(destination_y))
                    {
                        self.set(destination_x, destination_y, index);
                    }
                }
            }
        }
    }

    /// Returns the number of opaque pixels.
    #[must_use]
    pub fn opaque_count(&self) -> usize {
        self.pixels
            .iter()
            .filter(|index| **index != TRANSPARENT_INDEX)
            .count()
    }

    /// Returns the tight bounding box of opaque pixels, or `None` when the canvas is empty.
    ///
    /// # Panics
    ///
    /// Panics only if this canvas violates the dimension invariants established by
    /// [`Canvas::new`].
    #[must_use]
    pub fn opaque_bounds(&self) -> Option<Rect> {
        let mut minimum_x = i32::MAX;
        let mut minimum_y = i32::MAX;
        let mut maximum_x = i32::MIN;
        let mut maximum_y = i32::MIN;
        for y in 0..i32::try_from(self.height).expect("canvas height must fit i32") {
            for x in 0..i32::try_from(self.width).expect("canvas width must fit i32") {
                if self.get(x, y) != TRANSPARENT_INDEX {
                    minimum_x = minimum_x.min(x);
                    minimum_y = minimum_y.min(y);
                    maximum_x = maximum_x.max(x);
                    maximum_y = maximum_y.max(y);
                }
            }
        }
        if maximum_x < minimum_x {
            return None;
        }
        Some(Rect::new(
            minimum_x,
            minimum_y,
            u32::try_from(i64::from(maximum_x) - i64::from(minimum_x) + 1)
                .expect("opaque width must fit u32"),
            u32::try_from(i64::from(maximum_y) - i64::from(minimum_y) + 1)
                .expect("opaque height must fit u32"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_canvas_is_fully_transparent() {
        let canvas = Canvas::new(4, 4);

        assert_eq!(canvas.opaque_count(), 0);
        assert_eq!(canvas.opaque_bounds(), None);
    }

    #[test]
    fn writes_outside_the_canvas_are_ignored() {
        let mut canvas = Canvas::new(2, 2);

        canvas.set(-1, 0, 5);
        canvas.set(0, 9, 5);

        assert_eq!(canvas.opaque_count(), 0);
    }

    #[test]
    fn opaque_bounds_track_the_drawn_region() {
        let mut canvas = Canvas::new(8, 8);
        canvas.fill_rect(Rect::new(2, 3, 3, 2), 4);

        assert_eq!(canvas.opaque_bounds(), Some(Rect::new(2, 3, 3, 2)));
    }

    #[test]
    fn blit_preserves_destination_under_transparent_source_pixels() {
        let mut destination = Canvas::new(4, 4);
        destination.fill_rect(Rect::new(0, 0, 4, 4), 7);
        let mut source = Canvas::new(2, 2);
        source.set(0, 0, 9);

        destination.blit(&source, 1, 1);

        assert_eq!(destination.get(1, 1), 9);
        assert_eq!(destination.get(2, 2), 7);
    }
}
