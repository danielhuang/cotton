# Benchmark (using hyperfine)

Note: pnpm will fail due to missing peer dependencies. To ignore this warning:
```
echo 'strict-peer-dependencies=false' > .npmrc
```

## Benchmark commands

Without cache or lockfile

```
hyperfine --prepare 'rm -rf node_modules; pnpm store prune; yarn cache clean; npm cache clean --force; rm pnpm-lock.yaml; rm yarn.lock; rm package-lock.json; rm cotton.lock; true' 'yarn' 'pnpm i' 'cotton install' 'npm i'
```

Lockfile only

```
hyperfine --prepare 'rm -rf node_modules; pnpm store prune; yarn cache clean; npm cache clean --force; true' --warmup 1  'cotton install' 'pnpm i' 'yarn' 'npm i'
```

Lockfile and cache

```
hyperfine --prepare 'cotton clean; true' --warmup 1 'cotton install' && \
hyperfine --prepare 'rm -rf node_modules; true' --warmup 1 'yarn' && \
hyperfine --prepare 'rm -rf node_modules; true' --warmup 1 'npm i' && \
hyperfine --prepare 'rm -rf node_modules; true' --warmup 1 'pnpm i'
```

## create-react-app benchmark

These benchmarks were performed on [GitPod](https://gitpod.io/).

`package.json`:

```
{
  "name": "test",
  "version": "0.1.0",
  "private": true,
  "dependencies": {
    "@testing-library/jest-dom": "^5.16.5",
    "@testing-library/react": "^13.3.0",
    "@testing-library/user-event": "^13.5.0",
    "react": "^18.2.0",
    "react-dom": "^18.2.0",
    "react-scripts": "5.0.1",
    "web-vitals": "^2.1.4"
  },
  "scripts": {
    "start": "react-scripts start",
    "build": "react-scripts build",
    "test": "react-scripts test",
    "eject": "react-scripts eject"
  },
  "eslintConfig": {
    "extends": [
      "react-app",
      "react-app/jest"
    ]
  },
  "browserslist": {
    "production": [
      ">0.2%",
      "not dead",
      "not op_mini all"
    ],
    "development": [
      "last 1 chrome version",
      "last 1 firefox version",
      "last 1 safari version"
    ]
  }
}
```

### Results:

Without cache or lockfile:
```
Benchmark 1: yarn
  Time (mean ± σ):     40.895 s ±  7.631 s    [User: 30.729 s, System: 33.281 s]
  Range (min … max):   32.474 s … 56.217 s    10 runs
 
Benchmark 2: pnpm i
  Time (mean ± σ):     19.112 s ±  0.503 s    [User: 22.420 s, System: 12.527 s]
  Range (min … max):   18.587 s … 20.380 s    10 runs
 
Benchmark 3: cotton install
  Time (mean ± σ):      6.081 s ±  1.674 s    [User: 6.936 s, System: 10.250 s]
  Range (min … max):    4.213 s …  8.948 s    10 runs
 
Benchmark 4: npm i
  Time (mean ± σ):     45.730 s ±  1.407 s    [User: 48.774 s, System: 12.703 s]
  Range (min … max):   43.479 s … 48.153 s    10 runs
 
Summary
  'cotton install' ran
    3.14 ± 0.87 times faster than 'pnpm i'
    6.72 ± 2.24 times faster than 'yarn'
    7.52 ± 2.08 times faster than 'npm i'
```

Lockfile, cleared cache
```
Benchmark 1: cotton install
  Time (mean ± σ):      2.690 s ±  0.136 s    [User: 2.981 s, System: 9.243 s]
  Range (min … max):    2.491 s …  2.887 s    10 runs
 
Benchmark 2: pnpm i
  Time (mean ± σ):     21.516 s ±  6.232 s    [User: 22.188 s, System: 13.631 s]
  Range (min … max):   14.378 s … 35.494 s    10 runs
 
Benchmark 3: yarn
  Time (mean ± σ):     26.857 s ±  1.292 s    [User: 24.951 s, System: 29.473 s]
  Range (min … max):   25.523 s … 29.721 s    10 runs
 
Benchmark 4: npm i
  Time (mean ± σ):     28.406 s ±  0.487 s    [User: 39.482 s, System: 10.197 s]
  Range (min … max):   27.807 s … 29.121 s    10 runs
 
Summary
  'cotton install' ran
    8.00 ± 2.35 times faster than 'pnpm i'
    9.98 ± 0.70 times faster than 'yarn'
   10.56 ± 0.56 times faster than 'npm i'
```

Lockfile with cache:
```
Benchmark 1: cotton install
  Time (mean ± σ):      1.069 s ±  0.053 s    [User: 0.454 s, System: 1.539 s]
  Range (min … max):    1.012 s …  1.192 s    10 runs
 
Benchmark 1: yarn
  Time (mean ± σ):     13.885 s ±  2.046 s    [User: 11.187 s, System: 13.277 s]
  Range (min … max):   11.369 s … 16.752 s    10 runs
 
Benchmark 1: npm i
  Time (mean ± σ):     13.419 s ±  0.933 s    [User: 14.922 s, System: 8.799 s]
  Range (min … max):   12.044 s … 15.315 s    10 runs
 
Benchmark 1: pnpm i
  Time (mean ± σ):      5.511 s ±  0.318 s    [User: 4.963 s, System: 4.693 s]
  Range (min … max):    5.211 s …  6.113 s    10 runs
```