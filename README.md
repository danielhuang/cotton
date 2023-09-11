<div align="center">
	<img width="400" src="logo.svg">
</div>

<p align="center">
  Simple and speedy package manager
</p>

---

**Simple:** Cotton works with web applications, including those built with [React](https://reactjs.org/), [Next.js](https://nextjs.org/), [Vite](https://vitejs.dev/), [TypeScript](https://www.typescriptlang.org/), [ESLint](https://eslint.org/), and more.

**Speedy:** With a fast network, `cotton install` runs faster than `rm -rf node_modules`.

**Trouble free:** Ever run into errors when you forget to run `yarn` after pulling? No more. With Cotton, `node_modules` would never get out of sync.

**Quickstart** for [Netlify](#using-netlify) • [Cloudflare Pages](#using-cloudflare-pages)

## Installation

### Install from source

```
cargo install --locked --git https://github.com/danielhuang/cotton
```

### Install compiled artifact (Linux-only)

Download and install the compiled binary:

```
sudo curl -f#SL --compressed --tlsv1.2 https://api.cirrus-ci.com/v1/artifact/github/danielhuang/cotton/Build/binaries/target/x86_64-unknown-linux-gnu/release/cotton -o /usr/local/bin/cotton
sudo chmod +x /usr/local/bin/cotton
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

### Allow install scripts

If dependencies require install scripts (such as `puppeteer` or `electron`) to function, add this to `cotton.toml`:

```toml
allow_install_scripts = true
```

## Using as part of CI/CD?

In order to use Cotton, you have 2 options:
- Commit the binary to the repository, working similarly to committing Yarn 2+ (recommended)
- Download Cotton on-demand as part of the build process

If the binary is committed to the repository, use `./cotton` instead of `cotton`.

### Using Netlify

First, modify the configuration in `netlify.toml`, and add these lines:
```toml
[build.environment]
  NPM_FLAGS="--version"
```

Make sure `yarn.lock` is not present.

Add a command to remove `node_modules` from the build directory after the build finishes. Since Netlify stores its cache using tar, caching and extracting `node_modules` would be slower than reinstalling using Cotton.

For example, if the build command was

```sh
cotton run build
```

Add another command to the end:

```sh
cotton run build && mv node_modules _node_modules
```

This would disable Netlify's cache.

### Using Cloudflare Pages

Set the environment variable `NPM_FLAGS` to `--version`. Make sure that there is no `yarn.lock` in the repository.

## Comparison with other package managers

### [Bun's benchmark](https://github.com/oven-sh/bun/tree/main/bench/install)

| Tool | With lockfile and cache |
| --- | --- |
| Cotton | **0.272s** |
| Bun | 0.356s |
| pnpm | 2.332s |
| Yarn | 2.775s |
| npm | 4.309s |

### Installing packages used by [create-react-app](https://create-react-app.dev/):

![Hyperfine benchmark image](https://cdn.discordapp.com/attachments/355822466117009420/1080972798270390333/image.png)

| Tool | Initial install | With lockfile only | With lockfile and cache |
| --- | --- | --- | --- |
| Cotton | **4.0s** | **1.8s** | **0.3s** |
| pnpm | 24.3s | 17.9s | 5.5s |
| Yarn | 31.9s | 27.0s | 10.6s |
| npm | 35.4s | 21.3s | 13.0s |

See [benchmark](benchmark.md) for more information.

## Limitations

* Cotton does not currently support Git repositories or local paths as dependencies; only registries and direct urls are supported.
