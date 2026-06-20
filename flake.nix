{
  description = "Scarlet UI development environment";

  nixConfig = {
    extra-substituters = [ "https://scarlet-rust-toolchain.cachix.org" ];
    extra-trusted-public-keys = [
      "scarlet-rust-toolchain.cachix.org-1:p+coBExi0nNTIvWF/oM9H9/1/GhwFtqGZ2Vs+4pYl6o="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    scarlet-rust-toolchain.url = "github:petitstrawberry/scarlet-rust-nix";
  };

  outputs =
    { self, nixpkgs, scarlet-rust-toolchain }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs supportedSystems (system: f system);

      mkShell = system:
        let
          pkgs = import nixpkgs { inherit system; };
          rustToolchain = scarlet-rust-toolchain.packages.${system}.scarlet-rust-toolchain;

          rustHostTriple = {
            x86_64-linux = "x86_64-unknown-linux-gnu";
            aarch64-linux = "aarch64-unknown-linux-gnu";
            x86_64-darwin = "x86_64-apple-darwin";
            aarch64-darwin = "aarch64-apple-darwin";
          }.${system};

          rustBootstrapConfig = pkgs.writeText "scarlet-rust-bootstrap.toml" ''
            change-id = "ignore"

            [build]
            patch-binaries-for-nix = true

            [llvm]
            download-ci-llvm = false
          '';
        in
        pkgs.mkShell {
          packages = [
            rustToolchain
          ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
            # Font rendering for native desktop preview (winit + softbuffer)
            pkgs.fontconfig
            pkgs.dejavu_fonts
          ];

          CARGO_NET_GIT_FETCH_WITH_CLI = "true";
          RUST_BOOTSTRAP_CONFIG = "${rustBootstrapConfig}";
          SCARLET_RUST_HOST_TRIPLE = rustHostTriple;
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.stdenv.cc.cc.lib
          ];

          shellHook = ''
            export PATH="${rustToolchain}/bin:$PATH"
          '';
        };
    in
    {
      devShells = forAllSystems (system: {
        default = mkShell system;
      });
    };
}
