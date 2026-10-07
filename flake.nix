{
  description = "Environnement dev - CXSFM";

  inputs = {
    # Épinglé sur 25.05 : rustc 1.86 (OK pour l'edition 2024) + glibc 2.40.
    # Un toolchain plus récent (unstable = glibc 2.44) tire des symboles
    # libm re-versionnés (ex: atan2f@GLIBC_2.43) que le Steam Runtime
    # (sniper = glibc 2.36) ne fournit pas → LD_PRELOAD échoue et le jeu
    # ne démarre pas. Tout le shell DOIT venir de ce même nixpkgs pour
    # que le link final utilise glibc 2.40 (ne pas mixer deux stdenv).
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
  };

  outputs = { self, nixpkgs }: {
    devShells.x86_64-linux.default = let
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
    in pkgs.mkShell {
      buildInputs = with pkgs; [
        cargo
        rustc
        rustfmt
        clippy
        rust-analyzer
        # Dépendances requises par wgpu/iced sur Linux
        pkg-config
        vulkan-loader
        libxkbcommon
        wayland
      ];
      LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [ vulkan-loader libxkbcommon wayland ]);
    };
  };
}
