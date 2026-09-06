//! Logical texture uploads; native packet scheduling belongs to the backend.

use sgfx::backend::CommandExecutor;
use sgfx::ir::{CommandEncoder, ResourceTable, TextureId, TextureWrite};

use crate::{Error, FrameError, Stage};

/// Submit a complete texture update as one logical operation.
///
/// # Arguments
///
/// * `executor` - Executor bound to `resources`; a frame tracker retains receipts.
/// * `resources` - Logical resource descriptors.
/// * `texture` - Destination texture identifier.
/// * `write` - Complete source layout, validated before submission.
///
/// # Returns
///
/// Success after acceptance, or a lowering/execution error. A backend may split
/// the update into native packets, but owns their scheduling and completion.
/// Execution failure must not be blindly replayed. No GPU wait is performed
/// here, and no upload bytes are borrowed after execute returns.
pub fn upload_texture<E: CommandExecutor>(
    executor: &mut E,
    resources: &ResourceTable,
    texture: TextureId,
    write: TextureWrite<'_>,
) -> Result<(), FrameError<E::Error>> {
    let invalid = |_| Error::Sgfx(Stage::EncodeCommands);
    let texture = resources.texture_ref(texture).map_err(invalid)?;
    let mut encoder = CommandEncoder::new(resources);
    encoder.write_texture(texture, write).map_err(invalid)?;
    let commands = encoder.finish().map_err(invalid)?;
    executor.execute(&commands).map_err(FrameError::Execution)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use sgfx::ir::{
        Command, CommandBuffer, Extent2D, PixelRect, TextureDesc, TextureFormat, TextureUsage,
    };

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
    fn large_padded_upload_is_one_logical_operation_without_a_padded_final_row() {
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
        assert_eq!(recorder.0.len(), 1);
        assert_eq!(recorder.0[0].0, PixelRect::new(0, 0, 1024, 259).unwrap());
        assert_eq!(recorder.0[0].1, bytes);
    }

    #[test]
    fn invalid_complete_source_is_rejected_before_submission() {
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
        let bytes = alloc::vec![0; 512 * 1024];
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
