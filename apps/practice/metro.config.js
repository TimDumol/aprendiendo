const { getDefaultConfig } = require("expo/metro-config");

const config = getDefaultConfig(__dirname);

// expo-sqlite's web worker loads wa-sqlite as a Metro asset. Keeping the
// extension here makes the same repository usable in the browser and in a
// native development build; the app still uses its guarded localStorage path
// when a browser does not initialize SQLite.
config.resolver.assetExts = [...config.resolver.assetExts, "wasm"];

module.exports = config;
