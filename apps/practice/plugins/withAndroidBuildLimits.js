const { withAppBuildGradle, withGradleProperties } = require('@expo/config-plugins');

function positiveInteger(name, fallback) {
  const value = process.env[name] || fallback;
  if (!/^[1-9]\d*$/.test(value)) {
    throw new Error(`${name} must be a positive integer.`);
  }
  return value;
}

function memoryValue(name, fallback) {
  const value = process.env[name] || fallback;
  if (!/^[1-9]\d*[mMgG]$/.test(value)) {
    throw new Error(`${name} must be a memory value such as 2048m.`);
  }
  return value;
}

function property(items, key, value) {
  const existing = items.find((item) => item.type === 'property' && item.key === key);
  if (existing) {
    existing.value = value;
  } else {
    items.push({ type: 'property', key, value });
  }
}

function withAndroidBuildLimits(config) {
  const gradleWorkers = positiveInteger('PRACTICE_GRADLE_MAX_WORKERS', '1');
  const nativeCompileJobs = positiveInteger('PRACTICE_NATIVE_COMPILE_JOBS', '1');
  const heap = memoryValue('PRACTICE_GRADLE_HEAP', '2048m');
  const metaspace = memoryValue('PRACTICE_GRADLE_METASPACE', '1024m');
  const architectures = process.env.PRACTICE_ANDROID_ARCHITECTURES || 'arm64-v8a';
  if (!/^[a-z0-9-]+(,[a-z0-9-]+)*$/.test(architectures)) {
    throw new Error('PRACTICE_ANDROID_ARCHITECTURES must be a comma-separated ABI list.');
  }

  config = withGradleProperties(config, (mod) => {
    const jvmArgs = mod.modResults.find((item) => item.type === 'property' && item.key === 'org.gradle.jvmargs');
    const existingArgs = jvmArgs?.value || '';
    const boundedArgs = existingArgs
      .replace(/-Xmx\S+/g, '')
      .replace(/-XX:MaxMetaspaceSize=\S+/g, '')
      .trim();
    property(mod.modResults, 'org.gradle.jvmargs', `${boundedArgs} -Xmx${heap} -XX:MaxMetaspaceSize=${metaspace}`.trim());
    property(mod.modResults, 'org.gradle.parallel', 'false');
    property(mod.modResults, 'org.gradle.workers.max', gradleWorkers);
    property(mod.modResults, 'reactNativeArchitectures', architectures);
    return mod;
  });

  return withAppBuildGradle(config, (mod) => {
    if (mod.modResults.language !== 'groovy') return mod;
    const marker = '// Aprendiendo native build memory limits';
    if (mod.modResults.contents.includes(marker)) return mod;
    mod.modResults.contents += `

${marker}
// React Native's CMake builds invoke Ninja independently of Gradle workers.
// Keep native compilation bounded on developer laptops and CI workers.
android {
    defaultConfig {
        externalNativeBuild {
            cmake {
                arguments '-DCMAKE_JOB_POOLS=aprendiendo_native_compile=${nativeCompileJobs}',
                          '-DCMAKE_JOB_POOL_COMPILE=aprendiendo_native_compile'
            }
        }
    }
}
`;
    return mod;
  });
}

module.exports = withAndroidBuildLimits;
