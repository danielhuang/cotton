# Benchmark (using hyperfine)

Note: pnpm will fail due to missing peer dependencies. To ignore this warning:
```
echo 'strict-peer-dependencies=false' > .npmrc
```

## Benchmark commands

Without cache or lockfile

```
hyperfine --prepare 'rm -rf node_modules; pnpm store prune; yarn cache clean; npm cache clean --force; rm -rf .cotton; rm pnpm-lock.yaml; rm yarn.lock; rm package-lock.json; rm cotton.lock; true' 'yarn' 'pnpm i' 'cotton install' 'npm i'
```

Lockfile only

```
hyperfine --prepare 'rm -rf node_modules; pnpm store prune; yarn cache clean; npm cache clean --force; rm -rf .cotton; true' --warmup 1  'cotton install' 'pnpm i' 'yarn' 'npm i'
```

Lockfile and cache

```
hyperfine --prepare 'rm -rf node_modules; true' --warmup 1 'cotton install' 'pnpm i' 'yarn' 'npm i'
```

## create-react-app benchmark

These benchmarks were performed on [Gitpod](https://gitpod.io/).

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
  Time (mean ± σ):     31.887 s ±  0.398 s    [User: 33.810 s, System: 26.002 s]
  Range (min … max):   31.291 s … 32.583 s    10 runs

Benchmark 2: pnpm i
  Time (mean ± σ):     24.293 s ±  0.890 s    [User: 34.092 s, System: 9.305 s]
  Range (min … max):   23.104 s … 25.815 s    10 runs

Benchmark 3: cotton install
  Time (mean ± σ):      4.046 s ±  0.934 s    [User: 7.931 s, System: 8.355 s]
  Range (min … max):    3.228 s …  5.865 s    10 runs

Benchmark 4: npm i
  Time (mean ± σ):     35.365 s ±  0.338 s    [User: 43.839 s, System: 9.632 s]
  Range (min … max):   34.916 s … 36.194 s    10 runs

Summary
  'cotton install' ran
    6.00 ± 1.40 times faster than 'pnpm i'
    7.88 ± 1.82 times faster than 'yarn'
    8.74 ± 2.02 times faster than 'npm i'
```

Lockfile, cleared cache
```
Benchmark 1: cotton install
  Time (mean ± σ):      1.765 s ±  0.396 s    [User: 2.769 s, System: 6.609 s]
  Range (min … max):    1.468 s …  2.806 s    10 runs

Benchmark 2: pnpm i
  Time (mean ± σ):     17.913 s ±  0.301 s    [User: 25.494 s, System: 7.064 s]
  Range (min … max):   17.520 s … 18.326 s    10 runs

Benchmark 3: yarn
  Time (mean ± σ):     26.981 s ±  0.562 s    [User: 30.559 s, System: 25.649 s]
  Range (min … max):   25.976 s … 27.759 s    10 runs

Benchmark 4: npm i
  Time (mean ± σ):     21.264 s ±  0.354 s    [User: 30.141 s, System: 7.431 s]
  Range (min … max):   20.751 s … 21.870 s    10 runs

Summary
  'cotton install' ran
   10.15 ± 2.28 times faster than 'pnpm i'
   12.05 ± 2.71 times faster than 'npm i'
   15.29 ± 3.45 times faster than 'yarn'
```

Lockfile with cache:
```
Benchmark 1: cotton install
  Time (mean ± σ):     331.3 ms ±  45.4 ms    [User: 374.4 ms, System: 1433.5 ms]
  Range (min … max):   286.6 ms … 422.4 ms    10 runs

Benchmark 2: pnpm i
  Time (mean ± σ):      5.544 s ±  0.231 s    [User: 6.865 s, System: 2.780 s]
  Range (min … max):    5.232 s …  5.917 s    10 runs

Benchmark 3: yarn
  Time (mean ± σ):     10.566 s ±  0.162 s    [User: 11.866 s, System: 8.911 s]
  Range (min … max):   10.324 s … 10.847 s    10 runs

Benchmark 4: npm i
  Time (mean ± σ):     13.076 s ±  0.338 s    [User: 18.000 s, System: 5.566 s]
  Range (min … max):   12.471 s … 13.584 s    10 runs

Summary
  'cotton install' ran
   16.73 ± 2.40 times faster than 'pnpm i'
   31.89 ± 4.40 times faster than 'yarn'
   39.46 ± 5.50 times faster than 'npm i'
```
