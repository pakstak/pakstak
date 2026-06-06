# Pakstak

Pakstak is an experiment in "containers for apps" and "universal packaging".

It is inspired by Flatpak-style application isolation, but aims to be much simpler and to build on the shoulders of giants: OCI and Bubblewrap.

## Goals

- Work on any Linux distribution that has non-setuid Bubblewrap
- Reasonable and customizable isolation
- Be rootless, as much as possible
- Well-supported backing format: OCI images
- Few runtime and build-time dependencies
- Simplicity over feature completeness

## Non-Goals

- Being a replacement for Flatpak
- Native integration with non-universally available software

## Runtime Dependencies

- Non-setuid version of Bubblewrap

## Basic Usage

Install an image:

```sh
pakstak install alpine:latest
```

Run a command from an installed manifest:

```sh
pakstak run <manifest_hash> /bin/sh
```

Currently, it only uses per-user storage: `$HOME/.var/pakstak`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
