<div align="center">

# nix-ninja

Incremental compilation of [Ninja build files][ninja-build] using
[Nix Dynamic Derivations][dynamic-derivations].

Choosing ninja as the build graph representation lets us support any build
system that outputs ninja like CMake, meson, premake, gn, etc.

[Key features](#key-features) •
[Getting started](#getting-started) •
[Design notes][design notes] •
[Contributing](CONTRIBUTING.md)

</div>

## Key features

> [!IMPORTANT]
> There are still major todos, and depends on experimental features from an
> unreleased version of Nix. Come help us get nix-ninja to be useful day-to-day
> and working with an official Nix release!

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
NIX_NINJA=$(nix build --print-out-paths)
export PATH="${NIX_NINJA}/bin:$PATH"
# Meson respects this environment variable and uses as if its ninja.
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

## Contributing

We still have major TODOs, so would appreciate any help. We've organize them
under two [GitHub milestones][milestones]:

- `0.1.0` - The first release of `nix-ninja` aiming for correctness.
- `0.2.0` - Major performance features to make incremental builds productive.

Regardless, pull requests are welcome for any changes. Consider opening an issue
to discuss larger changes first, especially when the design space is large.

Please read [CONTRIBUTING](CONTRIBUTING.md) and the [design notes] so you
understand the big picture and prior art.

## License

The source code developed for nix-ninja is licensed under MIT License.

[design notes]: docs/design.md
[dynamic-derivations]: docs/dynamic-derivations.md
[milestones]: https://github.com/pdtpartners/nix-ninja/milestones
[ninja-build]: https://ninja-build.org/
[nix@d904921]: https://github.com/NixOS/nix/commit/d904921eecbc17662fef67e8162bd3c7d1a54ce0
