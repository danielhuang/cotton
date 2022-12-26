<div align="center">
	<img width="400" src="logo.svg">
</div>

<p align="center">
  Simple and speedy package manager
</p>

---

**Simple:** Cotton works with web applications, including those built with [React](https://reactjs.org/), [Next.js](https://nextjs.org/), [Vite](https://vitejs.dev/), [TypeScript](https://www.typescriptlang.org/), [ESLint](https://eslint.org/), and more.

**Speedy:** Cotton aggressively employs parallel operations, maximizing network and disk throughput, without sacrificing efficiency or simplicity. With a fast network, `cotton install` runs faster than `rm -rf node_modules`.

**Trouble free:** Inspired by [Cargo](https://crates.io/), Cotton ensures that all dependencies are installed correctly when performing operations or executing package scripts.

## Installation

### Install from source

```
cargo install --git https://github.com/danielhuang/cotton
```

### Install compiled artifact (Linux-only)

Download and install the compiled binary:

```
curl -f#SL --compressed --tlsv1.2 https://api.cirrus-ci.com/v1/artifact/github/danielhuang/cotton/Build/binaries/target/x86_64-unknown-linux-gnu/release/cotton > /usr/local/bin/cotton
chmod +x /usr/local/bin/cotton
```

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
| Cotton | **6.1s** | **2.7s** | **1.1s** |
| pnpm | 19.1s | 21.5s | 5.5s |
| Yarn | 40.9s | 26.8s | 13.9s |
| npm | 45.7s | 28.4s | 13.4s |

See [benchmark](benchmark.md) for more information.

Note: Cotton uses an internal per-project cache stored within `node_modules`. This is done to ensure that the cache is located in the same filesystem in order to allow hardlink installation.

## Limitations

* Cotton does not currently support Git repositories, direct urls, or local paths as dependencies
* Cotton does not execute `postinstall` scripts after installation