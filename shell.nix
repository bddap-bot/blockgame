# The pinned dev environment. `nix-shell --run 'cargo run --release'` and you are playing.
let
  pkgs = import (fetchTarball {
    url = "https://github.com/NixOS/nixpkgs/archive/d6c71932130818840fc8fe9509cf50be8c64634f.tar.gz";
    sha256 = "1klgyhj98j3gfsql5sn9rapyx62qk5g8adk5zh9mnc4d0fj61gdr";
  }) {};
in
pkgs.mkShell {
  buildInputs = with pkgs; [
    cargo
    rustc
    clippy
    rustfmt

    # bevy's linux build inputs: input devices, audio, vulkan, and both display stacks.
    pkg-config
    udev
    alsa-lib
    vulkan-loader
    libxkbcommon
    wayland
    libx11
    libxcursor
    libxi
    libxrandr
  ];

  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
    vulkan-loader
    udev
    alsa-lib
    libxkbcommon
    wayland
  ]);
}
