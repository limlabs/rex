// Rex programmatic server API
// Loads the native NAPI binding and re-exports createRex

import { createRequire } from 'node:module';
import { arch, platform } from 'node:process';

function loadNativeBinding() {
  const require = createRequire(import.meta.url);

  // Try platform-specific package first (for published npm packages)
  const platformPackage = `@limlabs/rex-${platform}-${arch}`;
  try {
    return require(platformPackage);
  } catch {
    // Fall through to local .node file (development)
  }

  // Try local .node file (built with napi build)
  try {
    return require('../rex-napi.node');
  } catch {
    // Fall through
  }

  // Try the build output from cargo
  try {
    return require('../../crates/rex_napi/rex-napi.node');
  } catch {
    // Fall through
  }

  throw new Error(
    `Failed to load Rex native binding. ` +
    `Tried: ${platformPackage}, ../rex-napi.node, ../../crates/rex_napi/rex-napi.node. ` +
    `Make sure the native module is built (cd crates/rex_napi && npm run build-debug).`
  );
}

const binding = loadNativeBinding();

/**
 * Create a new Rex application instance.
 *
 * Scans the pages directory, builds bundles, initializes the V8 isolate pool,
 * and returns a ready-to-use RexInstance.
 *
 * @param {import('./server').RexOptions} options
 * @returns {Promise<import('./server').RexInstance>}
 *
 * @example
 * ```js
 * import { createRex } from '@limlabs/rex/server'
 *
 * const rex = await createRex({ root: './my-app' })
 * const handle = rex.getRequestHandler()
 *
 * Bun.serve({ fetch: handle })
 * ```
 */
export const createRex = binding.createRex;
