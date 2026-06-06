# Pakstak

Pakstak is an experiment in "containers for apps" and "universal packaging".

It is inspired by Flatpak-style application isolation, but aims to be much simpler and to build on the shoulders of giants: OCI and Bubblewrap.

## Goals

- Work on any Linux distribution that has non-setuid Bubblewrap
- Reasonable and customizable isolation
- Be rootless, as much as possible
- Well-supported backing format (OCI images)
- Few runtime and build-time dependencies
- Simplicity over feature completeness

## Non-Goals

- Being a replacement for Flatpak
- Native integration with non-universally available software

## Issues

OCI image layers are extracted to the disk during installation.
While we have digest verification up to that point,
OCI images provide no standard way to verify the extracted rootfs
file structure.

Hopefully, this can be addressed later. For now, we just trust that the filesystem stores
the files correctly and that neither the user nor the OS modifies those files.
They are mounted read-only inside the container built for the app
and normally cannot be modified from inside the container.
If you have any suspicion that layer directories have been modified, the only option
for now is to reinstall them.

## Runtime Dependencies

- Non-setuid version of Bubblewrap

## Basic Usage

Install an image:

```sh
pakstak install my_alpine alpine:latest
```

Run a command from an installed manifest:

```sh
pakstak run my_alpine -- /bin/sh
```

Note that arguments after the first `--` are passed as-is to the Bubblewrap, so
you could define your own bindings and other parameters of the sandbox.

Update:

```sh
pakstak update
```

Currently, it only uses per-user storage: `$HOME/.var/pakstak`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
