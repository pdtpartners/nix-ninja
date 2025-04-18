{ 
  lib,
  pkgs,
  inputs,
  stdenv,
  runCommand,
  ...
}:
  let
    system = stdenv.system;

    ddPackages = inputs.nix-dynamic-derivations.packages.${system};

    nixDd = ddPackages.nix-everything;
    nixDdStatic = ddPackages.nix-everything-static;

    nixPortable = pkgs.callPackage inputs.nix-portable {
      inherit lib pkgs;

      buildSystem = system;
      dataDirName = ".nix-dd-portable";

      # The one from nixpkgs doesn't seem to work
      proot = import "${ inputs.nix-portable }/proot/alpine.nix" { inherit pkgs; };

      nix = nixDd;
      nixStatic = nixDdStatic;
      busybox = pkgs.pkgsStatic.busybox;
      bubblewrap = pkgs.pkgsStatic.bubblewrap;
      gnutar = pkgs.pkgsStatic.gnutar;
      perl = pkgs.pkgsBuildBuild.perl;
      xz = pkgs.pkgsStatic.xz;
      zstd = pkgs.pkgsStatic.zstd;
    };

    wrapped = runCommand "nix-bin" {} ''
      mkdir -p $out/bin

      ln -s ${ nixPortable }/bin/nix-portable $out/bin/nix

      ln -s $out/bin/nix $out/bin/nix-build  
      ln -s $out/bin/nix $out/bin/nix-channel  
      ln -s $out/bin/nix $out/bin/nix-collect-garbage  
      ln -s $out/bin/nix $out/bin/nix-copy-closure  
      ln -s $out/bin/nix $out/bin/nix-daemon  
      ln -s $out/bin/nix $out/bin/nix-env  
      ln -s $out/bin/nix $out/bin/nix-hash  
      ln -s $out/bin/nix $out/bin/nix-instantiate  
      ln -s $out/bin/nix $out/bin/nix-prefetch-url  
      ln -s $out/bin/nix $out/bin/nix-shell  
      ln -s $out/bin/nix $out/bin/nix-store
    '';
  in
    wrapped