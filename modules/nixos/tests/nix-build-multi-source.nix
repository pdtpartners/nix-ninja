{ self, pkgs, lib, ... }@args:

import ./nix-build.nix {
  flakeOutput = "example-multi-source";
  inputsFrom = [ pkgs.example-multi-source ];
  cmdline = "main";
  expectedStdout = "Hello multi-source example!";
} args
