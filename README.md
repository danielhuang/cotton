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

## Comparison with other package managers

Installing packages used by [create-react-app](https://create-react-app.dev/):

| Tool | Initial install | With lockfile only | With lockfile and cache |
| --- | --- | --- | --- |
| Cotton | 21.6s | 4.8s | 2.8s |
| Yarn | 27.6s | 13.7s | 3.7s |
| npm | 44.2s | 14.5s | 5.1s |

## Disadvantages

* Cotton does not currently support Git repositories, direct urls, or local paths as dependencies
* Cotton does not execute `postinstall` scripts after installation