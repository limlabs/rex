# Changelog

## [0.14.0](https://github.com/limlabs/rex/compare/v0.13.0...v0.14.0) (2026-03-10)


### Features

* add rex export command for static site generation ([#138](https://github.com/limlabs/rex/issues/138)) ([a0724af](https://github.com/limlabs/rex/commit/a0724af10834d639740fe9e893a670842c8f6dbe))
* **export:** add --base-path and fix docs CI ([#142](https://github.com/limlabs/rex/issues/142)) ([af36f9e](https://github.com/limlabs/rex/commit/af36f9e6ce4cd2ef1644b830be0e6d4b132f1728))
* **router:** add app router route.ts handler support ([#135](https://github.com/limlabs/rex/issues/135)) ([ef660a1](https://github.com/limlabs/rex/commit/ef660a14f4840949f59114fa4f63f66a2307784c))


### Bug Fixes

* **ci:** prevent beta-release double-fire from lockfile push ([#143](https://github.com/limlabs/rex/issues/143)) ([6d69f4b](https://github.com/limlabs/rex/commit/6d69f4bccc2ace2ed4f95e2ade374cd91d03cc45))
* **export:** rewrite navigation links with base path prefix ([#144](https://github.com/limlabs/rex/issues/144)) ([be15c8e](https://github.com/limlabs/rex/commit/be15c8edc9353abb45502140f85f995fe47b1551))
* inject CSS into RSC/app router HTML documents ([#147](https://github.com/limlabs/rex/issues/147)) ([4dc80f7](https://github.com/limlabs/rex/commit/4dc80f719e4b80844c9d8b0486ec3df9cac35f1e))


### Performance Improvements

* **ci:** use feature flags to speed up docs export ([#141](https://github.com/limlabs/rex/issues/141)) ([db0fbe9](https://github.com/limlabs/rex/commit/db0fbe9dd04897aeaea33b8edc1e287afda03939))
* **ci:** use feature flags to speed up docs site build ([#140](https://github.com/limlabs/rex/issues/140)) ([8f45889](https://github.com/limlabs/rex/commit/8f4588957bd85b703f5c15472b9706067fa087ad))

## [0.13.0](https://github.com/limlabs/rex/compare/v0.12.0...v0.13.0) (2026-03-09)


### Features

* cargo feature flags for compile-time tree-shaking ([#127](https://github.com/limlabs/rex/issues/127)) ([f31013d](https://github.com/limlabs/rex/commit/f31013d0a45f7cd7e06cb2faa20dc372a62e5d6e))
* **router:** support src/ directory and route-group-only layouts ([#132](https://github.com/limlabs/rex/issues/132)) ([fb1e6d4](https://github.com/limlabs/rex/commit/fb1e6d4d349a4b6abdcfd75130d1d2246b6f3180))


### Bug Fixes

* **build:** handle non-JS asset imports and wire app_dir correctly ([#134](https://github.com/limlabs/rex/issues/134)) ([191c240](https://github.com/limlabs/rex/commit/191c2407a26d11237e2a19bf8e4286defa88771a))
* **router:** pass app_dir to scan_project instead of hardcoding ([#133](https://github.com/limlabs/rex/issues/133)) ([644574c](https://github.com/limlabs/rex/commit/644574c3a04bd03852fccd6eb1fbac00f45edf6b))

## [0.12.0](https://github.com/limlabs/rex/compare/v0.11.0...v0.12.0) (2026-03-08)


### Features

* **ci:** add Railway deployment fixture and post-release deploy ([#86](https://github.com/limlabs/rex/issues/86)) ([54dc769](https://github.com/limlabs/rex/commit/54dc7691eadc83f901db11c43e7c3c8acb875737))


### Bug Fixes

* **ci:** fix cargo publish and smoke test failures in release workflow ([#122](https://github.com/limlabs/rex/issues/122)) ([e5fcb3a](https://github.com/limlabs/rex/commit/e5fcb3a8e3bffe3cdc480dda667610d290ef2a07))
* **ci:** prevent beta-release from double-triggering ([#125](https://github.com/limlabs/rex/issues/125)) ([3d52eee](https://github.com/limlabs/rex/commit/3d52eee3fda37f72a03671dd881a5cc6d0833914))
* **docker:** bind to 0.0.0.0 in production mode ([#124](https://github.com/limlabs/rex/issues/124)) ([4b553e7](https://github.com/limlabs/rex/commit/4b553e75644d033b68336082809b2df1e2a62949))
* **docker:** respect PORT env var for Railway ([#126](https://github.com/limlabs/rex/issues/126)) ([43dc967](https://github.com/limlabs/rex/commit/43dc967f10b988f22c687834c3d9117e4ce4646a))
* **docker:** split ENTRYPOINT/CMD to prevent argument doubling ([#96](https://github.com/limlabs/rex/issues/96)) ([7e25b72](https://github.com/limlabs/rex/commit/7e25b72b9168e4c662510fd56cab7bdee30ab0fd))

## [0.11.0](https://github.com/limlabs/rex/compare/v0.10.0...v0.11.0) (2026-03-08)


### Features

* add automatic static optimization for app router ([#92](https://github.com/limlabs/rex/issues/92)) ([bf3b787](https://github.com/limlabs/rex/commit/bf3b78712c051de3746aa42770ea87893048dc29))
* add automatic static optimization for pages router ([#88](https://github.com/limlabs/rex/issues/88)) ([aaa6d1a](https://github.com/limlabs/rex/commit/aaa6d1a6413e990e5e086b23d9d2f4d0ae527fb2))
* add next/font Google Fonts support with automatic optimization ([#83](https://github.com/limlabs/rex/issues/83)) ([b92874b](https://github.com/limlabs/rex/commit/b92874bd4c87bd8d49fccdea98a943e0bfd8e84c))
* add rex-py Python native extension via PyO3 ([#63](https://github.com/limlabs/rex/issues/63)) ([64bf67d](https://github.com/limlabs/rex/commit/64bf67d29949b8b117d679c5f7bf824feabb925a))
* **benchmarks:** add Vinext (Cloudflare) to benchmark suite ([#87](https://github.com/limlabs/rex/issues/87)) ([1cda419](https://github.com/limlabs/rex/commit/1cda419ab8c2f87fd7f4edc2a643497dc4c116d8))
* **build:** add MDX page support ([#69](https://github.com/limlabs/rex/issues/69)) ([ff0e6a0](https://github.com/limlabs/rex/commit/ff0e6a08ff3d92e4acb52e4544ea4c6bef7f817b))
* **rsc:** add generateMetadata / Metadata API for app router ([#84](https://github.com/limlabs/rex/issues/84)) ([764b325](https://github.com/limlabs/rex/commit/764b325aba33786cbe02b44deb8c658abfadd65e))
* **server:** add form server action support with encodeReply/decodeReply ([#64](https://github.com/limlabs/rex/issues/64)) ([87ca132](https://github.com/limlabs/rex/commit/87ca132d7fbde2272bb3553ce0b5201f9b2b6501))
* **v8:** add URLSearchParams polyfill and enhance URL ([#81](https://github.com/limlabs/rex/issues/81)) ([ada45c4](https://github.com/limlabs/rex/commit/ada45c4c30e7bcbabb7127c442fe2171e61f50b6))


### Bug Fixes

* address open CodeQL security alerts ([#91](https://github.com/limlabs/rex/issues/91)) ([36fc774](https://github.com/limlabs/rex/commit/36fc7741e3076b5c6f7fb46e2f0566a16cf4555b))
* **ci:** add merge_group trigger to unblock merge queue ([#89](https://github.com/limlabs/rex/issues/89)) ([6509ecd](https://github.com/limlabs/rex/commit/6509ecd39978948c02ec144ceef513193c233e0b))
* **dev:** include app/ directory in HMR file watcher ([#90](https://github.com/limlabs/rex/issues/90)) ([b9c470b](https://github.com/limlabs/rex/commit/b9c470bd507b8d16d964f1cfacc8a11308e2c8bb))
* **docker:** add rex_python to Dockerfile, exclude from coverage ([#80](https://github.com/limlabs/rex/issues/80)) ([c32b2ff](https://github.com/limlabs/rex/commit/c32b2ff1c4d12f97d59ac45754f0b43bc7d5bdf1))
* install fixtures/app-router deps in worktree setup ([#85](https://github.com/limlabs/rex/issues/85)) ([15d8725](https://github.com/limlabs/rex/commit/15d8725a65728c6abca808e1089e6db718ea5276))
* **rsc:** streaming head shell, __rex div, server action discovery ([#62](https://github.com/limlabs/rex/issues/62)) ([1c64906](https://github.com/limlabs/rex/commit/1c6490611a8b4206a0754b0a15ebccc3b028ce78))

## [0.10.0](https://github.com/limlabs/rex/compare/v0.9.0...v0.10.0) (2026-03-07)


### Features

* **build:** add Buffer polyfill for V8 SSR environment ([#49](https://github.com/limlabs/rex/issues/49)) ([bfd4f21](https://github.com/limlabs/rex/commit/bfd4f21c12415bcc5b93322919ad16d09e4cd4ed))
* **build:** add React Server Functions ("use server") support ([#53](https://github.com/limlabs/rex/issues/53)) ([13805f0](https://github.com/limlabs/rex/commit/13805f065c9332acae6123969ac8bf6ba62a42bd))
* **cli:** use Rust oxc_linter crate instead of npm oxlint binary ([#52](https://github.com/limlabs/rex/issues/52)) ([8933c7f](https://github.com/limlabs/rex/commit/8933c7fb271635cfcb6d438ba9b548dbf9a5f7f4))
* **lint:** dogfood rex lint on fixtures and benchmarks ([#58](https://github.com/limlabs/rex/issues/58)) ([1d0476e](https://github.com/limlabs/rex/commit/1d0476e15d9680c315d413e4bcf7d6aed93a8290))
* **v8:** polyfill crypto with randomUUID and createHash ([#68](https://github.com/limlabs/rex/issues/68)) ([ed94b3b](https://github.com/limlabs/rex/commit/ed94b3bf90dfbeac60abe679d6103fd183995a41))


### Bug Fixes

* **build:** add @tailwindcss/cli for Tailwind v4 compatibility ([#74](https://github.com/limlabs/rex/issues/74)) ([dc46191](https://github.com/limlabs/rex/commit/dc4619175cc228fa05bc0bf744c86c488e0814e7))
* **ci:** allow any edits to grandfathered files in length check ([#73](https://github.com/limlabs/rex/issues/73)) ([2906290](https://github.com/limlabs/rex/commit/290629097208fc74f1cf43d47dd50e312a8d6812))
* **ci:** prevent beta release double-trigger on Cargo.lock update ([#48](https://github.com/limlabs/rex/issues/48)) ([462cb18](https://github.com/limlabs/rex/commit/462cb184fc68815de8dea480c82a4b66e79fc4bb))
* **ci:** skip file-length check for grandfathered files ([#72](https://github.com/limlabs/rex/issues/72)) ([b19ed80](https://github.com/limlabs/rex/commit/b19ed806e1c9756c1478f7114fb35a659580fabc))
* **ci:** wait for npm package propagation before smoke tests ([#78](https://github.com/limlabs/rex/issues/78)) ([69764af](https://github.com/limlabs/rex/commit/69764aff2946e210e8009bc8d3476cf996eb734b))
* **hooks:** prevent lockfile mutation in pre-commit hooks ([#59](https://github.com/limlabs/rex/issues/59)) ([e4e5880](https://github.com/limlabs/rex/commit/e4e58807ff7505d5a09e5ee7a1237d9a39473d44))
* run npm and cargo installs on worktree creation ([#57](https://github.com/limlabs/rex/issues/57)) ([ca5f2ae](https://github.com/limlabs/rex/commit/ca5f2aed9d69c76def76096def629fdb4eeebd6c))

## [0.9.0](https://github.com/limlabs/rex/compare/v0.8.0...v0.9.0) (2026-03-03)


### Features

* **build:** embed React for zero-config projects ([#42](https://github.com/limlabs/rex/issues/42)) ([310b73a](https://github.com/limlabs/rex/commit/310b73aac3f67916ab95f337842c0ceafebd2741))
* **cli:** add `rex fmt` command using oxc_formatter ([#40](https://github.com/limlabs/rex/issues/40)) ([ec29bce](https://github.com/limlabs/rex/commit/ec29bce1f52008414b6db3b54f9ed5d246d26e72))


### Bug Fixes

* **build:** extract builtin React into project node_modules/ ([#47](https://github.com/limlabs/rex/issues/47)) ([ffd471c](https://github.com/limlabs/rex/commit/ffd471cd6b4612aa6174dea79a3b997dd2f6459c))
* **cli:** include v8::console in default log filter ([#46](https://github.com/limlabs/rex/issues/46)) ([4d77b45](https://github.com/limlabs/rex/commit/4d77b4588d9330c46635b85000256cc7bec48eb3))

## [0.8.0](https://github.com/limlabs/rex/compare/v0.7.0...v0.8.0) (2026-03-03)


### Features

* **build:** embed runtime files and distribute platform binaries via npm ([#44](https://github.com/limlabs/rex/issues/44)) ([c3e4c94](https://github.com/limlabs/rex/commit/c3e4c9469efe4f0718d7143d85bd18f21af0535f))
* **build:** enable sourcemap generation ([#34](https://github.com/limlabs/rex/issues/34)) ([12e930f](https://github.com/limlabs/rex/commit/12e930fea99e33027405b2dd1f535cbe856b7ff9))
* **ci:** add strict TypeScript type checking for fixtures ([#35](https://github.com/limlabs/rex/issues/35)) ([bee139b](https://github.com/limlabs/rex/commit/bee139b587ca6f9abe0aa35c02dfc9185c13b519))
* platform-specific CLI binary distribution via npm ([#41](https://github.com/limlabs/rex/issues/41)) ([d1609b4](https://github.com/limlabs/rex/commit/d1609b489b10bf8a8d72edb1d852f1a217a7c922))
* **v8:** polyfill path module for server bundles ([#30](https://github.com/limlabs/rex/issues/30)) ([1cb9dcd](https://github.com/limlabs/rex/commit/1cb9dcd31b0dd7657d61d389f40c1c3b400d4b23))


### Bug Fixes

* **ci:** use RELEASE_PAT to trigger beta release workflow ([#38](https://github.com/limlabs/rex/issues/38)) ([1a55a54](https://github.com/limlabs/rex/commit/1a55a540ff0c5ad54c62325c3c00bf755434fa50))
* **cli:** show logs on TUI startup failure ([#33](https://github.com/limlabs/rex/issues/33)) ([2c041d6](https://github.com/limlabs/rex/commit/2c041d65c9a76ec9a9d18cdfe126a9703f4b2d3d))
* **cli:** strip TypeScript annotations from HMR client ([#43](https://github.com/limlabs/rex/issues/43)) ([faa6421](https://github.com/limlabs/rex/commit/faa64213b29521cac577207bc9b51903158f81e3))
* prevent PAT from leaking into chat output ([#39](https://github.com/limlabs/rex/issues/39)) ([f1e9437](https://github.com/limlabs/rex/commit/f1e94370343441147d4a947eccdb90f348b6224c))
* remove broken WorktreeCreate hook ([#31](https://github.com/limlabs/rex/issues/31)) ([02506c3](https://github.com/limlabs/rex/commit/02506c34e342180a9c2230c37b25ae659cdf5ae6))
* replace npx oxlint with direct binary ([#37](https://github.com/limlabs/rex/issues/37)) ([e591502](https://github.com/limlabs/rex/commit/e591502a34468a848472fd56e3db355d53f6afca))
* resolve dependabot security vulnerabilities ([#36](https://github.com/limlabs/rex/issues/36)) ([c0c7dcc](https://github.com/limlabs/rex/commit/c0c7dcc1e72e2cc360b8497f779a9133d1d7016b))

## [0.7.0](https://github.com/limlabs/rex/compare/v0.6.0...v0.7.0) (2026-03-02)


### Features

* **ci:** add coverage ratchet pre-commit hook ([#28](https://github.com/limlabs/rex/issues/28)) ([cba5ed2](https://github.com/limlabs/rex/commit/cba5ed207d8da9c3d76f3a1bc690ec7fecb61d0d))
* **v8:** polyfill process.env from Rust environment variables ([#27](https://github.com/limlabs/rex/issues/27)) ([dd90c78](https://github.com/limlabs/rex/commit/dd90c78a89653b5bdf0fad3e61e7817964504305))

## [0.6.0](https://github.com/limlabs/rex/compare/v0.5.1...v0.6.0) (2026-03-02)


### Features

* **v8:** add Node.js fs module polyfill for server-side code ([#20](https://github.com/limlabs/rex/issues/20)) ([efb85d6](https://github.com/limlabs/rex/commit/efb85d6ea785f9d74bbec19ac11b16a4187f881e))

## [0.5.1](https://github.com/limlabs/rex/compare/v0.5.0...v0.5.1) (2026-03-02)


### Bug Fixes

* add missing rex/head module ([#24](https://github.com/limlabs/rex/issues/24)) ([194329b](https://github.com/limlabs/rex/commit/194329bc34b89ff001b852c0d24c9ee7c3b9b6b4))
* **ci:** filter artifact download to exclude docker metadata ([#21](https://github.com/limlabs/rex/issues/21)) ([c9a1900](https://github.com/limlabs/rex/commit/c9a1900e16783954ca61dddae3a78588c7691ad9))

## [0.5.0](https://github.com/limlabs/rex/compare/v0.4.0...v0.5.0) (2026-03-02)


### Features

* **core:** support TOML config as alternative to JSON ([#18](https://github.com/limlabs/rex/issues/18)) ([2246fd9](https://github.com/limlabs/rex/commit/2246fd96ce0c491948e4b110f8a47a0bfce71b7c))

## [0.4.0](https://github.com/limlabs/rex/compare/v0.3.0...v0.4.0) (2026-03-02)


### Features

* configure bot identity for Claude commits and PRs ([800b0c9](https://github.com/limlabs/rex/commit/800b0c949b51b06810e58cde4e09fc139566a6af))
* convention-based MCP tool handlers ([#6](https://github.com/limlabs/rex/issues/6)) ([31dc8a7](https://github.com/limlabs/rex/commit/31dc8a7adf38d7e23edeb970082e3ea864ae7105))
* sandboxed agent workflow with conventional commit enforcement ([#8](https://github.com/limlabs/rex/issues/8)) ([7957e62](https://github.com/limlabs/rex/commit/7957e6231a9a3e696a4329088965e8e3d200c2c0))


### Bug Fixes

* resolve oxlint warnings and deny future warnings ([#14](https://github.com/limlabs/rex/issues/14)) ([79ed7f1](https://github.com/limlabs/rex/commit/79ed7f18999a645c71c43ad6a5b5b28b7b6ad486))

## [0.3.0](https://github.com/limlabs/rex/compare/v0.2.1...v0.3.0) (2026-03-01)


### Features

* add Linux ARM64 release build ([21d4a89](https://github.com/limlabs/rex/commit/21d4a8935c797e5553c950495e241093c91d4e54))


### Bug Fixes

* remove nonexistent rex_auth crate from Dockerfile ([e0f4706](https://github.com/limlabs/rex/commit/e0f47063a6e9d088538d64501b53d0351d583948))

## [0.2.1](https://github.com/limlabs/rex/compare/v0.2.0...v0.2.1) (2026-03-01)


### Bug Fixes

* fix release workflow conditions and macOS runner ([44c46cb](https://github.com/limlabs/rex/commit/44c46cb88d1c927dc6f394d3fb83ac9f31015f66))
* merge release jobs into release-please workflow ([b1e5d38](https://github.com/limlabs/rex/commit/b1e5d387b40e6ce99ccbd00e4700a0475456da7f))

## [0.2.0](https://github.com/limlabs/rex/compare/v0.1.0...v0.2.0) (2026-03-01)


### Features

* add automated release pipeline with release-please ([3b9d4db](https://github.com/limlabs/rex/commit/3b9d4dbe40272380441f7d3ae2516a423dc27f57))
