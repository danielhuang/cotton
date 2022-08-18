<div align="center">
	<img width="400" src="logo.svg">
</div>

<p align="center">
  Simple and speedy package manager
</p>

---

**Simple:** Cotton works with web applications, including those built with [React](https://reactjs.org/) or [Next.js](https://nextjs.org/).

**Speedy:** Cotton aggressively employs parallel operations, maximizing network and disk throughput, without sacrificing efficiency or simplicity.

**Trouble free:** Inspired by [Cargo](https://crates.io/), Cotton ensures that all dependencies are installed correctly when performing operations or executing package scripts.

## Installation

### Install from source

```
cargo install --git https://github.com/danielhuang/cotton
```

### Install compiled artifact (Linux-only)

Download and install the compiled binary:

```
curl -f#SL --compressed --tlsv1.2 https://api.cirrus-ci.com/v1/artifact/github/danielhuang/cotton/Build/binaries/target/release/cotton > /usr/local/bin/cotton
chmod +x /usr/local/bin/cotton
```

If glibc is not installed or is not a recent version, download and install a statically linked (musl) artifact:

```
curl -f#SL https://nightly.link/danielhuang/cotton/workflows/build/master/Binary.zip > cotton.zip
unzip -o cotton.zip
unzip -o cotton_null_x86_64-unknown-linux-musl.zip
```

This will install to the current directory.

## Get started

### Install packages

```
cotton install
```

This will install packages to `node_modules` and save `cotton.lock` if needed.

### Run a script

To run the `start` script:

```
cotton run start
```

To automatically restart the script when `package.json` changes:

```
cotton run start --watch package.json
```

Unlike other package managers, Cotton does not require installing packages before running scripts. Missing packages will be installed on-demand automatically.

### Update package versions

```
cotton update
```

This will load the latest available versions of dependencies (including transitive dependencies) and save registry information to `cotton.lock`. Specified versions in `package.json` are not modified.

## Comparison with other package managers

Installing packages used by [create-react-app](https://create-react-app.dev/):

| Tool | Initial install | With lockfile only | With lockfile and cache |
| --- | --- | --- | --- |
| Cotton | **3.9s** | **1.8s** | **0.3s** |
| pnpm | 11.6s | 7.8s | 2.1s |
| Yarn | 26.5s | 13.6s | 3.7s |
| npm | 43.5s | 14.5s | 4.9s |

Note: Cotton uses an internal per-project cache stored within `node_modules`. This is done to ensure that the cache is located in the same filesystem in order to allow hardlink installation.

## Limitations

* Cotton does not currently support Git repositories, direct urls, or local paths as dependencies
* Cotton does not execute `postinstall` scripts after installation