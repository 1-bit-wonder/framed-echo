{
  description = "framed-echo: zero-copy length-prefixed framing codec and async echo server over Tokio";

  # Single pinned input. `nix flake lock` records the exact nixpkgs revision in
  # flake.lock, so `nix develop` / `nix build` are byte-for-byte reproducible.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));
    in
    {
      # `nix build` — builds the library and both binaries from the pinned
      # crate versions in Cargo.lock (each crate's source is content-hashed).
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "framed-echo";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          # Pure Rust; no native/system dependencies.
        };
      });

      # `nix develop` — a shell with the toolchain and helpers on PATH. Same
      # rustc/cargo for everyone, no rustup, no host toolchain drift.
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
            pkgs.rust-analyzer
            pkgs.cargo-nextest
          ];
          # Point rust-analyzer at the matching std sources.
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixpkgs-fmt);
    };
}
