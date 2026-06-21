//! Buffer - Pixel buffer management for ScarletUI
//!
//! Provides BGRA format pixel buffers with alpha blending support.

use crate::color::Color;
use crate::geometry::Size;
use alloc::vec;
use alloc::vec::Vec;

/// Pixel buffer in BGRA format
///
/// Each pixel is stored as a u32 in BGRA byte order:
/// - Byte 0: Blue
/// - Byte 1: Green
/// - Byte 2: Red
/// - Byte 3: Alpha
#[derive(Clone)]
pub struct Buffer {
    width: u32,
    height: u32,
    logical_width: u32,
    logical_height: u32,
    scale_milli: u32,
    data: Vec<u32>,
}

impl Buffer {
    /// Create a new buffer with the given size
    pub fn new(size: Size) -> Self {
        let width = size.width as u32;
        let height = size.height as u32;
        Self {
            width,
            height,
            logical_width: width,
            logical_height: height,
            scale_milli: 1000,
            data: vec![0; (width * height) as usize],
        }
    }

    /// Create a buffer with explicit dimensions
    pub fn from_dimensions(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            logical_width: width,
            logical_height: height,
            scale_milli: 1000,
            data: vec![0; (width * height) as usize],
        }
    }

    /// Create a physical buffer for a logical size using the current UI scale.
    pub fn from_logical_dimensions(logical_width: u32, logical_height: u32) -> Self {
        Self::from_logical_dimensions_with_scale(
            logical_width,
            logical_height,
            crate::graphics::current_scale_milli(),
        )
    }

    /// Create a physical buffer for a logical size using an explicit UI scale.
    pub fn from_logical_dimensions_with_scale(
        logical_width: u32,
        logical_height: u32,
        scale_milli: u32,
    ) -> Self {
        let scale_milli = scale_milli.max(1);
        let width = Self::scale_len(logical_width, scale_milli);
        let height = Self::scale_len(logical_height, scale_milli);
        Self {
            width,
            height,
            logical_width,
            logical_height,
            scale_milli,
            data: vec![0; (width * height) as usize],
        }
    }

    fn scale_len(value: u32, scale_milli: u32) -> u32 {
        ((value as u64)
            .saturating_mul(scale_milli as u64)
            .saturating_add(999)
            / 1000)
            .max(1) as u32
    }

    /// Get the buffer width
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Get the buffer height
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Get the logical buffer width.
    pub fn logical_width(&self) -> u32 {
        self.logical_width
    }

    /// Get the logical buffer height.
    pub fn logical_height(&self) -> u32 {
        self.logical_height
    }

    /// Get the scale used to create this buffer in milli-units.
    pub fn scale_milli(&self) -> u32 {
        self.scale_milli
    }

    /// Get the buffer size
    pub fn size(&self) -> Size {
        Size {
            width: self.width as f32,
            height: self.height as f32,
        }
    }

    /// Get the pixel data as a slice
    pub fn as_slice(&self) -> &[u32] {
        &self.data
    }

    /// Get the pixel data as a mutable slice
    pub fn as_mut_slice(&mut self) -> &mut [u32] {
        &mut self.data
    }

    /// Clear the buffer with a color
    pub fn clear(&mut self, color: Color) {
        self.data.fill(color.to_bgra());
    }

    /// Clear a rectangle with a color
    pub fn clear_rect(&mut self, x: u32, y: u32, width: u32, height: u32, color: Color) {
        if width == 0 || height == 0 {
            return;
        }
        let pixel = color.to_bgra();
        let x_end = (x + width).min(self.width);
        let y_end = (y + height).min(self.height);
        for yy in y..y_end {
            let row_start = (yy * self.width + x) as usize;
            let row_end = (yy * self.width + x_end) as usize;
            self.data[row_start..row_end].fill(pixel);
        }
    }

    /// Composite another buffer into this buffer
    ///
    /// # Arguments
    /// * `src` - Source buffer to composite
    /// * `dst_x` - Destination X position
    /// * `dst_y` - Destination Y position
    /// * `opacity` - Opacity multiplier (0.0 - 1.0)
    pub fn composite(&mut self, src: &Buffer, dst_x: i32, dst_y: i32, opacity: f32) {
        let src_w = src.width as i32;
        let src_h = src.height as i32;
        let dst_w = self.width as i32;
        let dst_h = self.height as i32;

        let dst_left = dst_x.max(0);
        let dst_top = dst_y.max(0);
        let dst_right = (dst_x + src_w).min(dst_w);
        let dst_bottom = (dst_y + src_h).min(dst_h);

        if dst_right <= dst_left || dst_bottom <= dst_top {
            return;
        }

        let fully_opaque = opacity >= 1.0;

        for target_y in dst_top..dst_bottom {
            let src_y = (target_y - dst_y) as usize;
            let target_x = dst_left;
            let width = (dst_right - dst_left) as usize;
            let src_row_start = src_y * src.width as usize;
            let dst_row_start = target_y as usize * self.width as usize;
            let src_offset = (target_x - dst_x) as usize;

            if fully_opaque {
                let src_slice =
                    &src.data[src_row_start + src_offset..src_row_start + src_offset + width];
                let dst_slice = &mut self.data
                    [dst_row_start + target_x as usize..dst_row_start + target_x as usize + width];

                if src_slice.iter().all(|&p| (p >> 24) == 0xFF) {
                    dst_slice.copy_from_slice(src_slice);
                    continue;
                }
            }

            for dx in 0..width {
                let src_pixel = src.data[src_row_start + src_offset + dx];
                let dst_idx = dst_row_start + target_x as usize + dx;
                self.data[dst_idx] = Self::blend_pixels(self.data[dst_idx], src_pixel, opacity);
            }
        }
    }

    /// Composite a source rectangle from another buffer into this buffer.
    pub fn composite_rect(
        &mut self,
        src: &Buffer,
        src_x: i32,
        src_y: i32,
        src_w: i32,
        src_h: i32,
        dst_x: i32,
        dst_y: i32,
        opacity: f32,
    ) {
        self.composite_rect_clipped(
            src,
            src_x,
            src_y,
            src_w,
            src_h,
            dst_x,
            dst_y,
            opacity,
            0,
            0,
            self.width as i32,
            self.height as i32,
        );
    }

    /// Composite a source rectangle from another buffer with a clip rect in destination space.
    pub fn composite_rect_clipped(
        &mut self,
        src: &Buffer,
        src_x: i32,
        src_y: i32,
        src_w: i32,
        src_h: i32,
        dst_x: i32,
        dst_y: i32,
        opacity: f32,
        clip_x: i32,
        clip_y: i32,
        clip_w: i32,
        clip_h: i32,
    ) {
        if src_w <= 0 || src_h <= 0 || clip_w <= 0 || clip_h <= 0 {
            return;
        }

        let dst_left = dst_x.max(clip_x).max(0).max(dst_x - src_x);
        let dst_top = dst_y.max(clip_y).max(0).max(dst_y - src_y);
        let dst_right = (dst_x + src_w)
            .min(clip_x + clip_w)
            .min(self.width as i32)
            .min(dst_x + src.width as i32 - src_x);
        let dst_bottom = (dst_y + src_h)
            .min(clip_y + clip_h)
            .min(self.height as i32)
            .min(dst_y + src.height as i32 - src_y);

        if dst_right <= dst_left || dst_bottom <= dst_top {
            return;
        }

        let fully_opaque = opacity >= 1.0;

        for target_y in dst_top..dst_bottom {
            let source_y = src_y + (target_y - dst_y);
            let src_row_start = source_y as usize * src.width as usize;
            let dst_row_start = target_y as usize * self.width as usize;
            let source_x = src_x + (dst_left - dst_x);
            let width = (dst_right - dst_left) as usize;

            if fully_opaque {
                let src_slice = &src.data
                    [src_row_start + source_x as usize..src_row_start + source_x as usize + width];
                let dst_slice = &mut self.data
                    [dst_row_start + dst_left as usize..dst_row_start + dst_left as usize + width];

                if src_slice.iter().all(|&p| (p >> 24) == 0xFF) {
                    dst_slice.copy_from_slice(src_slice);
                    continue;
                }
            }

            for dx in 0..width {
                let src_pixel = src.data[src_row_start + source_x as usize + dx];
                let dst_idx = dst_row_start + dst_left as usize + dx;
                self.data[dst_idx] = Self::blend_pixels(self.data[dst_idx], src_pixel, opacity);
            }
        }
    }

    /// Composite another buffer into this buffer with a clip rect in destination space.
    pub fn composite_clipped(
        &mut self,
        src: &Buffer,
        dst_x: i32,
        dst_y: i32,
        opacity: f32,
        clip_x: i32,
        clip_y: i32,
        clip_w: i32,
        clip_h: i32,
    ) {
        if clip_w <= 0 || clip_h <= 0 {
            return;
        }

        let src_w = src.width as i32;
        let src_h = src.height as i32;
        let dst_w = self.width as i32;
        let dst_h = self.height as i32;

        let dst_left = dst_x.max(clip_x).max(0);
        let dst_top = dst_y.max(clip_y).max(0);
        let dst_right = (dst_x + src_w).min(clip_x + clip_w).min(dst_w);
        let dst_bottom = (dst_y + src_h).min(clip_y + clip_h).min(dst_h);

        if dst_right <= dst_left || dst_bottom <= dst_top {
            return;
        }

        let fully_opaque = opacity >= 1.0;

        for target_y in dst_top..dst_bottom {
            let src_y = (target_y - dst_y) as usize;
            let target_x = dst_left;
            let width = (dst_right - dst_left) as usize;
            let src_row_start = src_y * src.width as usize;
            let dst_row_start = target_y as usize * self.width as usize;
            let src_offset = (target_x - dst_x) as usize;

            if fully_opaque {
                let src_slice =
                    &src.data[src_row_start + src_offset..src_row_start + src_offset + width];
                let dst_slice = &mut self.data
                    [dst_row_start + target_x as usize..dst_row_start + target_x as usize + width];
                if src_slice.iter().all(|&p| (p >> 24) == 0xFF) {
                    dst_slice.copy_from_slice(src_slice);
                    continue;
                }
            }

            for dx in 0..width {
                let src_pixel = src.data[src_row_start + src_offset + dx];
                let dst_idx = dst_row_start + target_x as usize + dx;
                self.data[dst_idx] = Self::blend_pixels(self.data[dst_idx], src_pixel, opacity);
            }
        }
    }

    /// Composite another buffer into this buffer with a rounded clip rect in destination space.
    pub fn composite_clipped_rounded(
        &mut self,
        src: &Buffer,
        dst_x: i32,
        dst_y: i32,
        opacity: f32,
        clip_x: i32,
        clip_y: i32,
        clip_w: i32,
        clip_h: i32,
        radius: f32,
    ) {
        if clip_w <= 0 || clip_h <= 0 {
            return;
        }
        if radius <= 0.0 {
            self.composite_clipped(src, dst_x, dst_y, opacity, clip_x, clip_y, clip_w, clip_h);
            return;
        }

        let src_w = src.width as i32;
        let src_h = src.height as i32;
        let dst_w = self.width as i32;
        let dst_h = self.height as i32;

        let dst_left = dst_x.max(clip_x).max(0);
        let dst_top = dst_y.max(clip_y).max(0);
        let dst_right = (dst_x + src_w).min(clip_x + clip_w).min(dst_w);
        let dst_bottom = (dst_y + src_h).min(clip_y + clip_h).min(dst_h);

        if dst_right <= dst_left || dst_bottom <= dst_top {
            return;
        }

        let max_radius = (clip_w.min(clip_h) as f32) / 2.0;
        let radius = radius.max(0.0).min(max_radius);
        let radius_sq = radius * radius;

        for target_y in dst_top..dst_bottom {
            let src_y = target_y - dst_y;
            let src_row = (src_y * src.width as i32) as usize;
            let dst_row = (target_y * self.width as i32) as usize;
            let center_y = (target_y - clip_y) as f32 + 0.5;
            let dy_top = center_y;
            let dy_bottom = (clip_h as f32) - center_y;
            for target_x in dst_left..dst_right {
                let center_x = (target_x - clip_x) as f32 + 0.5;
                let dx_left = center_x;
                let dx_right = (clip_w as f32) - center_x;

                let in_center = (dx_left >= radius && dx_right >= radius)
                    || (dy_top >= radius && dy_bottom >= radius);
                if !in_center {
                    let corner_dx = radius - dx_left.min(dx_right);
                    let corner_dy = radius - dy_top.min(dy_bottom);
                    let dist_sq = corner_dx * corner_dx + corner_dy * corner_dy;
                    if dist_sq > radius_sq {
                        let dist = libm::sqrtf(dist_sq);
                        let coverage = (radius + 0.5 - dist).max(0.0).min(1.0);
                        if coverage <= 0.0 {
                            continue;
                        }
                        let src_x = target_x - dst_x;
                        let src_pixel = src.data[src_row + src_x as usize];
                        let dst_idx = dst_row + target_x as usize;
                        self.data[dst_idx] =
                            Self::blend_pixels(self.data[dst_idx], src_pixel, opacity * coverage);
                        continue;
                    }
                }

                let src_x = target_x - dst_x;
                let src_pixel = src.data[src_row + src_x as usize];
                let dst_idx = dst_row + target_x as usize;
                self.data[dst_idx] = Self::blend_pixels(self.data[dst_idx], src_pixel, opacity);
            }
        }
    }

    /// Composite a source rectangle with a rounded clip rect in destination space.
    pub fn composite_rect_clipped_rounded(
        &mut self,
        src: &Buffer,
        src_x: i32,
        src_y: i32,
        src_w: i32,
        src_h: i32,
        dst_x: i32,
        dst_y: i32,
        opacity: f32,
        clip_x: i32,
        clip_y: i32,
        clip_w: i32,
        clip_h: i32,
        radius: f32,
    ) {
        if radius <= 0.0 {
            self.composite_rect_clipped(
                src, src_x, src_y, src_w, src_h, dst_x, dst_y, opacity, clip_x, clip_y, clip_w,
                clip_h,
            );
            return;
        }
        if src_w <= 0 || src_h <= 0 || clip_w <= 0 || clip_h <= 0 {
            return;
        }

        let dst_left = dst_x.max(clip_x).max(0).max(dst_x - src_x);
        let dst_top = dst_y.max(clip_y).max(0).max(dst_y - src_y);
        let dst_right = (dst_x + src_w)
            .min(clip_x + clip_w)
            .min(self.width as i32)
            .min(dst_x + src.width as i32 - src_x);
        let dst_bottom = (dst_y + src_h)
            .min(clip_y + clip_h)
            .min(self.height as i32)
            .min(dst_y + src.height as i32 - src_y);

        if dst_right <= dst_left || dst_bottom <= dst_top {
            return;
        }

        let max_radius = (clip_w.min(clip_h) as f32) / 2.0;
        let radius = radius.max(0.0).min(max_radius);
        let radius_sq = radius * radius;

        for target_y in dst_top..dst_bottom {
            let source_y = src_y + (target_y - dst_y);
            let src_row = source_y as usize * src.width as usize;
            let dst_row = target_y as usize * self.width as usize;
            let center_y = (target_y - clip_y) as f32 + 0.5;
            let dy_top = center_y;
            let dy_bottom = (clip_h as f32) - center_y;
            for target_x in dst_left..dst_right {
                let center_x = (target_x - clip_x) as f32 + 0.5;
                let dx_left = center_x;
                let dx_right = (clip_w as f32) - center_x;
                let mut coverage = 1.0;

                let in_center = (dx_left >= radius && dx_right >= radius)
                    || (dy_top >= radius && dy_bottom >= radius);
                if !in_center {
                    let corner_dx = radius - dx_left.min(dx_right);
                    let corner_dy = radius - dy_top.min(dy_bottom);
                    let dist_sq = corner_dx * corner_dx + corner_dy * corner_dy;
                    if dist_sq > radius_sq {
                        let dist = libm::sqrtf(dist_sq);
                        coverage = (radius + 0.5 - dist).max(0.0).min(1.0);
                        if coverage <= 0.0 {
                            continue;
                        }
                    }
                }

                let source_x = src_x + (target_x - dst_x);
                let src_pixel = src.data[src_row + source_x as usize];
                let dst_idx = dst_row + target_x as usize;
                self.data[dst_idx] =
                    Self::blend_pixels(self.data[dst_idx], src_pixel, opacity * coverage);
            }
        }
    }

    /// Blend two pixels with alpha
    ///
    /// Pixel format: 0xAARRGGBB in memory, becomes BGRA in little-endian
    pub(crate) fn blend_pixels(dst: u32, src: u32, opacity: f32) -> u32 {
        let dst_bytes = dst.to_le_bytes();
        let src_bytes = src.to_le_bytes();

        let src_a = src_bytes[3] as f32;
        if src_a == 255.0 && opacity >= 1.0 {
            return src;
        }
        if src_a == 0.0 || opacity <= 0.0 {
            return dst;
        }

        let alpha = src_a * opacity;
        let inv_a = 255.0 - alpha;

        let b = (src_bytes[0] as f32 * alpha + dst_bytes[0] as f32 * inv_a) / 255.0;
        let g = (src_bytes[1] as f32 * alpha + dst_bytes[1] as f32 * inv_a) / 255.0;
        let r = (src_bytes[2] as f32 * alpha + dst_bytes[2] as f32 * inv_a) / 255.0;
        let a = dst_bytes[3] as f32 + (255.0 - dst_bytes[3] as f32) * alpha / 255.0;

        let bytes = [
            b.clamp(0.0, 255.0) as u8,
            g.clamp(0.0, 255.0) as u8,
            r.clamp(0.0, 255.0) as u8,
            a.clamp(0.0, 255.0) as u8,
        ];
        u32::from_le_bytes(bytes)
    }

    /// Set a pixel at the given position
    ///
    /// # Arguments
    /// * `x` - X position
    /// * `y` - Y position
    /// * `pixel` - Pixel value in BGRA format
    pub fn set_pixel(&mut self, x: u32, y: u32, pixel: u32) {
        if x < self.width && y < self.height {
            self.data[(y * self.width + x) as usize] = pixel;
        }
    }

    /// Get a pixel at the given position
    ///
    /// Returns None if position is out of bounds
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<u32> {
        if x < self.width && y < self.height {
            Some(self.data[(y * self.width + x) as usize])
        } else {
            None
        }
    }

    /// Fill a rectangle with a color
    pub fn fill_rect(&mut self, x: u32, y: u32, width: u32, height: u32, color: Color) {
        let pixel = color.to_bgra();
        for dy in 0..height {
            for dx in 0..width {
                let px = x + dx;
                let py = y + dy;
                if px < self.width && py < self.height {
                    self.data[(py * self.width + px) as usize] = pixel;
                }
            }
        }
    }

    /// Get the pixel data as a u8 slice (BGRA format)
    ///
    /// This converts the internal u32 slice to a u8 slice for compatibility
    /// with drawing APIs that expect byte-level access.
    pub fn data(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.data.as_ptr() as *const u8, self.data.len() * 4) }
    }

    /// Get the pixel data as a mutable u8 slice (BGRA format)
    ///
    /// This converts the internal u32 slice to a u8 slice for compatibility
    /// with drawing APIs that expect byte-level access.
    pub fn data_mut(&mut self) -> &mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(self.data.as_mut_ptr() as *mut u8, self.data.len() * 4)
        }
    }
}
