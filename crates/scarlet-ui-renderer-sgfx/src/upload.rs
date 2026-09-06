//! Bounded independent texture uploads shared by UI and compositor consumers.

use sgfx::backend::CommandExecutor;
use sgfx::ir::{CommandEncoder, PixelRect, ResourceTable, TextureId, TextureWrite};

use crate::{Error, FrameError, Stage};

/// Pixel-byte budget per logical upload, leaving room for native packet headers.
pub const MAX_UPLOAD_BYTES: usize = 512 * 1024;

/// Submit a texture update as ordered, bounded row strips.
///
/// # Arguments
///
/// * `executor` - Executor bound to `resources`; a frame tracker retains receipts.
/// * `resources` - Logical resource descriptors.
/// * `texture` - Destination texture identifier.
/// * `write` - Complete source layout, validated before submitting any strip.
///
/// # Returns
///
/// Success after all strips are accepted, or a lowering/execution error. Earlier
/// strips may have been accepted on execution failure; do not blindly replay.
/// No GPU wait is performed here, and no bytes are borrowed after each execute.
pub fn upload_texture<E: CommandExecutor>(
    executor: &mut E,
    resources: &ResourceTable,
    texture: TextureId,
    write: TextureWrite<'_>,
) -> Result<(), FrameError<E::Error>> {
    let invalid = |_| Error::Sgfx(Stage::EncodeCommands);
    let texture = resources.texture_ref(texture).map_err(invalid)?;
    // Validate the entire source and destination before accepting a prefix.
    let mut validation = CommandEncoder::new(resources);
    validation.write_texture(texture, write).map_err(invalid)?;
    let descriptor = resources.texture(texture).map_err(invalid)?;
    let area = write.destination();
    let row_bytes = area.width() as usize * descriptor.format().bytes_per_pixel() as usize;
    let stride = write.bytes_per_row() as usize;
    // Native alpha-only formats expand to four-byte pixels during lowering.
    let rows_per_strip = (MAX_UPLOAD_BYTES / (area.width() as usize * 4)).max(1) as u32;
    let mut y = 0;
    while y < area.height() {
        let rows = rows_per_strip.min(area.height() - y);
        let start = y as usize * stride;
        let length = (rows as usize - 1) * stride + row_bytes;
        let bytes = write
            .data()
            .get(start..start + length)
            .ok_or(Error::InvalidFrame)?;
        let strip = PixelRect::new(area.x(), area.y() + y, area.width(), rows).map_err(invalid)?;
        let write = TextureWrite::new(strip, write.bytes_per_row(), bytes).map_err(invalid)?;
        let mut encoder = CommandEncoder::new(resources);
        encoder.write_texture(texture, write).map_err(invalid)?;
        let commands = encoder.finish().map_err(invalid)?;
        executor.execute(&commands).map_err(FrameError::Execution)?;
        y += rows;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use sgfx::ir::{Command, CommandBuffer, Extent2D, TextureDesc, TextureFormat, TextureUsage};

    #[derive(Default)]
    struct Recorder(Vec<(PixelRect, Vec<u8>)>);
    impl CommandExecutor for Recorder {
        type Error = ();
        fn execute<'r, 'data>(&mut self, commands: &CommandBuffer<'r, 'data>) -> Result<(), ()> {
            assert_eq!(commands.commands().len(), 1);
            let Command::WriteTexture { write, .. } = commands.commands()[0] else {
                panic!("upload")
            };
            self.0.push((write.destination(), write.data().to_vec()));
            Ok(())
        }
    }

    #[test]
    fn large_padded_uploads_are_split_in_order_without_a_padded_final_row() {
        let table = ResourceTable::new();
        let texture = table
            .define_texture(
                TextureDesc::new(
                    TextureFormat::Bgra8Unorm,
                    Extent2D::new(1024, 259).unwrap(),
                    TextureUsage::COPY_DST,
                )
                .unwrap(),
            )
            .unwrap()
            .id();
        let stride = 4104;
        let bytes: Vec<u8> = (0..stride * 258 + 4096).map(|index| index as u8).collect();
        let mut recorder = Recorder::default();
        upload_texture(
            &mut recorder,
            &table,
            texture,
            TextureWrite::new(
                PixelRect::new(0, 0, 1024, 259).unwrap(),
                stride as u32,
                &bytes,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(recorder.0.len(), 3);
        let mut y = 0;
        for (area, data) in recorder.0 {
            assert_eq!(area.y(), y);
            assert!(area.width() as usize * area.height() as usize * 4 <= MAX_UPLOAD_BYTES);
            let start = y as usize * stride;
            assert_eq!(data, bytes[start..start + data.len()]);
            y += area.height();
        }
        assert_eq!(y, 259);
    }

    #[test]
    fn invalid_complete_source_is_rejected_before_any_strip() {
        let table = ResourceTable::new();
        let texture = table
            .define_texture(
                TextureDesc::new(
                    TextureFormat::Bgra8Unorm,
                    Extent2D::new(1024, 259).unwrap(),
                    TextureUsage::COPY_DST,
                )
                .unwrap(),
            )
            .unwrap()
            .id();
        let mut recorder = Recorder::default();
        let bytes = alloc::vec![0; MAX_UPLOAD_BYTES];
        assert!(
            upload_texture(
                &mut recorder,
                &table,
                texture,
                TextureWrite::new(PixelRect::new(0, 0, 1024, 259).unwrap(), 4096, &bytes,).unwrap()
            )
            .is_err()
        );
        assert!(recorder.0.is_empty());
    }
}
