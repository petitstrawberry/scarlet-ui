use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

/// Single-shot native preview host (default).
#[derive(Parser, Debug)]
#[command(name = "scarlet-ui-preview", bin_name = "scarlet-ui-preview")]
pub struct RunArgs {
    #[arg(long)]
    pub manifest_path: PathBuf,
    #[arg(long)]
    pub source: Option<PathBuf>,
    #[arg(long)]
    pub target: Option<String>,
    #[arg(long)]
    pub features: Option<String>,
    #[arg(long)]
    pub preview: Option<String>,
    #[arg(long, default_value_t = 250, value_name = "MILLIS")]
    pub poll_ms: u64,
    #[arg(long, value_name = "SCALE")]
    pub force_scale: Option<f32>,
    #[arg(long)]
    pub build_only: bool,
    #[arg(
        long,
        hide = true,
        help = "Deprecated no-op; PaintCommand rendering is enabled by default"
    )]
    pub paint: bool,
    #[cfg(feature = "gpu")]
    #[arg(long, default_value_t = true)]
    pub gpu: bool,
    #[cfg(feature = "gpu")]
    #[arg(long)]
    pub no_gpu: bool,
}

/// IPC server mode (invoked as `scarlet-ui-preview serve ...`).
#[derive(Parser, Debug, Clone)]
#[command(name = "scarlet-ui-preview-serve", bin_name = "scarlet-ui-preview")]
pub struct ServeArgs {
    #[arg(long)]
    pub manifest_path: PathBuf,
    #[arg(long)]
    pub source: Option<PathBuf>,
    #[arg(long)]
    pub target: Option<String>,
    #[arg(long)]
    pub features: Option<String>,
    #[arg(long)]
    pub preview: Option<String>,
    #[arg(long, default_value_t = 250, value_name = "MILLIS")]
    pub poll_ms: u64,
    #[arg(long, value_name = "SCALE")]
    pub force_scale: Option<f32>,
    #[cfg(feature = "gpu")]
    #[arg(long, default_value_t = true)]
    pub gpu: bool,
    #[cfg(feature = "gpu")]
    #[arg(long)]
    pub no_gpu: bool,
}

impl RunArgs {
    pub fn poll_interval(&self) -> Duration {
        Duration::from_millis(self.poll_ms.max(16))
    }

    pub fn force_scale_milli(&self) -> Option<u32> {
        self.force_scale.map(scale_to_milli)
    }

    #[cfg(feature = "gpu")]
    pub fn use_gpu(&self) -> bool {
        self.gpu && !self.no_gpu
    }
}

impl ServeArgs {
    pub fn poll_interval(&self) -> Duration {
        Duration::from_millis(self.poll_ms.max(16))
    }

    pub fn force_scale_milli(&self) -> Option<u32> {
        self.force_scale.map(scale_to_milli)
    }

    #[cfg(feature = "gpu")]
    pub fn use_gpu(&self) -> bool {
        self.gpu && !self.no_gpu
    }
}

fn scale_to_milli(scale: f32) -> u32 {
    (scale.max(0.001) * 1000.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::{RunArgs, ServeArgs};
    use clap::Parser;

    #[test]
    fn run_args_parse_with_manifest_path() {
        let args = RunArgs::try_parse_from(["scarlet-ui-preview", "--manifest-path", "X"]);

        assert!(args.is_ok());
    }

    #[test]
    fn run_args_parse_fails_without_manifest_path() {
        let args = RunArgs::try_parse_from(["scarlet-ui-preview"]);

        assert!(args.is_err());
    }

    #[test]
    fn run_args_parse_build_only() {
        let args =
            RunArgs::try_parse_from(["scarlet-ui-preview", "--manifest-path", "X", "--build-only"])
                .expect("run args should parse");

        assert!(args.build_only);
    }

    #[test]
    fn run_args_parse_poll_ms_without_clamping() {
        let args = RunArgs::try_parse_from([
            "scarlet-ui-preview",
            "--manifest-path",
            "X",
            "--poll-ms",
            "10",
        ])
        .expect("run args should parse");

        assert_eq!(args.poll_ms, 10);
    }

    #[test]
    fn serve_args_parse_with_manifest_path() {
        let args = ServeArgs::try_parse_from(["scarlet-ui-preview", "--manifest-path", "X"]);

        assert!(args.is_ok());
    }

    #[test]
    fn serve_args_reject_build_only() {
        let args = ServeArgs::try_parse_from([
            "scarlet-ui-preview",
            "--manifest-path",
            "X",
            "--build-only",
        ]);

        assert!(args.is_err());
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn run_args_enable_gpu_by_default_when_feature_is_enabled() {
        let args = RunArgs::try_parse_from(["scarlet-ui-preview", "--manifest-path", "X"])
            .expect("run args should parse");

        assert!(args.use_gpu());
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn run_args_no_gpu_disables_gpu() {
        let args =
            RunArgs::try_parse_from(["scarlet-ui-preview", "--manifest-path", "X", "--no-gpu"])
                .expect("run args should parse");

        assert!(!args.use_gpu());
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn serve_args_enable_gpu_by_default_when_feature_is_enabled() {
        let args = ServeArgs::try_parse_from(["scarlet-ui-preview", "--manifest-path", "X"])
            .expect("serve args should parse");

        assert!(args.use_gpu());
    }
}
