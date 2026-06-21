use std::thread;
use std::time::Duration;

use scarlet_ui::preview::PreviewHost;

use crate::build::{CargoStdout, PreviewTarget, build_and_load, prepare_project, print_previews};
use crate::cli::RunArgs;
use crate::watch::latest_source_mtime;

pub fn run(args: RunArgs) -> Result<(), String> {
    let project = prepare_project(&args)?;

    println!("[preview] package={}", project.package_name);
    println!("[preview] manifest={}", project.manifest_path.display());
    if let PreviewTarget::Source { source_path, .. } = &project.target {
        println!("[preview] source={}", source_path.display());
    }

    if args.build_only {
        let library = build_and_load(&args, &project, 0, CargoStdout::Inherit)?;
        print_previews(&library);
        println!("[preview] build loaded");
        return Ok(());
    }

    let mut last_seen = latest_source_mtime(&project.crate_dir)?;
    let mut build_index = 0u64;
    let mut host: Option<PreviewHost> = None;
    let mut backend = scarlet_ui::WinitBackend::new();

    match build_and_load(&args, &project, build_index, CargoStdout::Inherit) {
        Ok(library) => {
            print_previews(&library);
            println!("[preview] initial build loaded");
            host = Some(PreviewHost::new_with_backend(
                library,
                args.preview.as_deref(),
                &mut backend,
            )?);
        }
        Err(error) => {
            eprintln!("[preview] initial build failed: {error}");
        }
    }

    #[cfg(feature = "gpu")]
    if args.use_gpu() {
        if let Some(ref mut host) = host {
            crate::gpu::setup_gpu_present(host);
        }
    }

    loop {
        let current_mtime = latest_source_mtime(&project.crate_dir)?;
        if current_mtime > last_seen {
            last_seen = current_mtime;
            build_index = build_index.wrapping_add(1);
            println!("[preview] change detected; rebuilding");
            match build_and_load(&args, &project, build_index, CargoStdout::Inherit) {
                Ok(library) => {
                    print_previews(&library);
                    if let Some(host) = host.as_mut() {
                        host.reload(library)?;
                        println!("[preview] reloaded");
                    } else {
                        host = Some(PreviewHost::new_with_backend(
                            library,
                            args.preview.as_deref(),
                            &mut backend,
                        )?);
                        println!("[preview] loaded");
                    }
                }
                Err(error) => {
                    eprintln!("[preview] rebuild failed; keeping previous preview: {error}");
                }
            }
        }

        if let Some(host) = host.as_mut() {
            if !host.tick(Duration::from_millis(16))? {
                break;
            }
        } else {
            thread::sleep(args.poll_interval());
        }
    }

    Ok(())
}
