{
  description = "Environnement dev - CXSFM";

  inputs = {
    # Base 25.05 : glibc 2.40 + link final. Le Steam Runtime (sniper =
    # glibc 2.36) ne fournit pas les symboles libm re-versionnés des
    # glibc récentes (ex: atan2f@GLIBC_2.43) → le LINK final doit rester
    # sur le cc/glibc 2.40 de ce nixpkgs (ne pas mixer deux stdenv
    # pour le link).
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    # Toolchain Rust récente (MSRV egui 0.36 = rustc 1.95) : SEUL le
    # toolchain vient d'ici, tout le reste (link, loaders) reste sur
    # 25.05 ci-dessus. Rev pinnée par flake.lock (reproductible
    # jusqu'au prochain `nix flake update nixpkgs-toolchain`).
    # Après chaque bump, vérifier les symboles du binaire :
    #   objdump -T target/release/libcxsfm.so | grep -oP "GLIBC_[0-9.]+"
    # Référence build rustup 1.99 : max GLIBC_2.35 (< sniper 2.36).
    nixpkgs-toolchain.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs, nixpkgs-toolchain }: {
    devShells.x86_64-linux.default = let
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
      pkgsToolchain = nixpkgs-toolchain.legacyPackages.x86_64-linux;
    in pkgs.mkShell {
      buildInputs = [
        pkgsToolchain.cargo
        pkgsToolchain.rustc
        pkgsToolchain.rustfmt
        pkgsToolchain.clippy
        pkgsToolchain.rust-analyzer
      ];
    };
  };
}
