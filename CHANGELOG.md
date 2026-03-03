# Changelog

## [0.7.1](https://github.com/limlabs/rex/compare/v0.7.0...v0.7.1) (2026-03-03)


### Bug Fixes

* **ci:** use RELEASE_PAT to trigger beta release workflow ([#38](https://github.com/limlabs/rex/issues/38)) ([1a55a54](https://github.com/limlabs/rex/commit/1a55a540ff0c5ad54c62325c3c00bf755434fa50))
* **cli:** show logs on TUI startup failure ([#33](https://github.com/limlabs/rex/issues/33)) ([2c041d6](https://github.com/limlabs/rex/commit/2c041d65c9a76ec9a9d18cdfe126a9703f4b2d3d))
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
