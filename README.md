<div align="center">

# nix-ninja

Incremental compilation of [Ninja build files][ninja-build] using
[Nix Dynamic Derivations][dynamic-derivations].

Choosing ninja as the build graph representation lets us support any build
system that outputs ninja like CMake, meson, premake, gn, etc.

</div>

## Key features

> [!IMPORTANT]
> This is currently a work in progress, and dynamic derivations is not
> stabilized yet.

- Parses `ninja.build` files and generates a derivation per compilation unit.
- Stores build inputs & outputs in content-addressed derivations for granular
  and Nix-native incrementality.
- Compatible CLI for ninja, so if you set `$NINJA` to `nix-ninja` then meson
  just works.
- Supports running locally (which runs `nix build` on your behalf), or inside a
  Nix derivation (which creates dynamic derivations).

## Getting started

First you need to use [nix@d904921] and enable the following experimental
features:

```sh
experimental-features = ["nix-command" "dynamic-derivations" "ca-derivations" "recursive-nix"]
```

Then you can try building the examples:

```sh
# Builds a basic main.cpp.
nix build github:pdtpartners/nix-ninja#example-hello

# Builds a basic main.cpp with dependency inference for its header.
nix build github:pdtpartners/nix-ninja#example-header

# Builds Nix 2.27.1.
nix build github:pdtpartners/nix-ninja#example-nix
```

You can also try running `nix-ninja` outside of Nix, but you'll need both
`nix-ninja` and `nix-ninja-task` to be in your `$PATH`. Make sure
`nix-ninja-task` is from the `/nix/store` as it is needed inside derivations
`nix-ninja` generates.

```sh
NIX_NINJA=$(nix build --print-out-paths .#nix-ninja)
NIX_NINJA_TASK=$(nix build --print-out-paths .#nix-ninja-task)
export PATH="${NIX_NINJA}/bin:${NIX_NINJA_TASK}/bin:$PATH"
# Meson respects this environment variable and uses it as if it's ninja.
export NINJA="${NIX_NINJA}/bin/nix-ninja"
```

Then you can go to an example and run it like so:
```sh
$ nix-shell
$ cd examples/hello
$ meson setup build
$ cd build
$ meson compile hello
$ ./hello
Hello Nix dynamic derivations!
```

## License

The source code developed for nix-ninja is licensed under MIT License.

[dynamic-derivations]: docs/dynamic-derivations.md
[ninja-build]: https://ninja-build.org/
[nix@d904921]: https://github.com/NixOS/nix/commit/d904921eecbc17662fef67e8162bd3c7d1a54ce0
