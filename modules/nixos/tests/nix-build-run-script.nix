{ self, pkgs, lib, ... }@args:

import ./nix-build.nix {
  flakeOutput = "example-run-script";
  inputsFrom = [ pkgs.example-run-script ];
  cmdline = "main";
  expectedStdout = "Hello run-script example!";
} args
