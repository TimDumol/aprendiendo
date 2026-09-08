# Aprendiendo speaking practice

The app keeps the existing browser MVP and adds the mobile-first practice foundation described in [`docs/recording-app/mobile-plan.md`](../../docs/recording-app/mobile-plan.md). Native builds store task, round, analysis, outbox, and recording metadata in SQLite and move finalized audio into private document storage. Web remains a labeled experiment with the loopback MVP.

Native coaching uses the authenticated durable practice API. Set `EXPO_PUBLIC_PRACTICE_API_URL` to the HTTPS base URL of the main Rust service. The Settings screen completes the Pocket ID OAuth PKCE browser flow; the access token is stored in platform secure storage under `aprendiendo.practice.access-token`. The mobile client never defaults to a phone loopback address. Offline coaching is written to the local outbox and retried after restart; provider use remains controlled by the Settings consent and spending fields.

The API worker reserves `PRACTICE_ANALYSIS_RESERVATION_USD` per paid job against the monthly `PRACTICE_MONTHLY_SPEND_CAP_USD` server cap (defaults are `$0.10` and `$1.00`). A cap rejection preserves the local recording and does not silently retry or submit a second paid request.

The bundled image-description tasks use project-generated local practice visuals, with provenance and SHA-256 checksums in [`assets/tasks/README.md`](assets/tasks/README.md). They are not official DELE photographs.

## Native development build

From this directory, install dependencies and build the native development client with the platform toolchain:

```sh
npm install
npx expo run:ios
npx expo run:android
```

To build a standalone release APK, install it with ADB, and launch it on one
connected Android device or emulator from the repository root:

```sh
adb devices
EXPO_PUBLIC_PRACTICE_API_URL=https://mars.timdumol.com \
  scripts/build-install-practice-android.sh --build --phone
```

The script generates the Android project when needed, runs the Gradle release
build, installs `app-release.apk` with ADB, and launches package
`com.timdumol.aprendiendo.practice`. Use `--debug` for a debug APK, `--phone`
to ignore emulators, `--device SERIAL` to select a target, or `--apk PATH` to
choose the APK location. `PRACTICE_ANDROID_BUILD=1` and
`PRACTICE_ANDROID_DEVICE` provide the equivalent environment overrides.

The build has OOM protection by default: one Gradle worker, one native
CMake/Ninja compile job, `-Xmx2048m`, `MaxMetaspaceSize=1024m`, disabled Gradle
parallelism, and only the connected target ABI (ARM64 for a physical phone).
Override these deliberately with `PRACTICE_GRADLE_MAX_WORKERS`,
`PRACTICE_NATIVE_COMPILE_JOBS`, `PRACTICE_GRADLE_HEAP`,
`PRACTICE_GRADLE_METASPACE`, or `PRACTICE_ANDROID_ARCHITECTURES`.

The app API URL and OAuth values are bundled at build time. The Android build
script supplies the production values by default, including client
`aprendiendo-practice-mobile`, resource `https://mars.timdumol.com`, and
redirect `aprendiendo-practice-mvp://oauth/callback`. Never put `GEMINI_API_KEY`
or any other provider secret in an `EXPO_PUBLIC_*` variable.

For a production-connected build, the script can be run directly:

```sh
export EXPO_PUBLIC_PRACTICE_API_URL=https://mars.timdumol.com
export EXPO_PUBLIC_OAUTH_ISSUER=https://auth.aries.timdumol.com
export EXPO_PUBLIC_OAUTH_AUTHORIZATION_URL=https://auth.aries.timdumol.com/authorize
export EXPO_PUBLIC_OAUTH_TOKEN_URL=https://auth.aries.timdumol.com/api/oidc/token
export EXPO_PUBLIC_OAUTH_CLIENT_ID=aprendiendo-practice-mobile
export EXPO_PUBLIC_OAUTH_SCOPE="openid profile email learning:access"
export EXPO_PUBLIC_OAUTH_REDIRECT_URI=aprendiendo-practice-mvp://oauth/callback
export EXPO_PUBLIC_OAUTH_RESOURCE=https://mars.timdumol.com
scripts/build-install-practice-android.sh
```

The Pocket ID client is public, PKCE enabled, unrestricted by user group, and
granted the user delegated `learning:access` permission for the production API.
The idempotent setup command is `scripts/configure-practice-pocket-id.sh`; it
reads the admin key from the ignored root file `.pocket-id-api-key`, never
prints it, and can be rerun after restoring Pocket ID. Offline recording and
local history do not need that client.

From the repository root, deploy the production API and practice worker with:

```sh
scripts/release.sh
```

The release script reads `apps/practice/.env.mcp` for the server-side
`GEMINI_API_KEY` and `GEMINI_MODEL`, unless `APRENDIENDO_PRACTICE_ENV_FILE`
selects another ignored env file. It does not bundle those values into the
Android app.

For a staging OAuth-enabled native build, configure the nonsecret values before starting Expo:

```dotenv
EXPO_PUBLIC_PRACTICE_API_URL=https://practice.example.test
EXPO_PUBLIC_OAUTH_ISSUER=https://auth.example.test
EXPO_PUBLIC_OAUTH_AUTHORIZATION_URL=https://auth.example.test/authorize
EXPO_PUBLIC_OAUTH_TOKEN_URL=https://auth.example.test/api/oidc/token
EXPO_PUBLIC_OAUTH_CLIENT_ID=mobile-practice
EXPO_PUBLIC_OAUTH_SCOPE=learning:access
EXPO_PUBLIC_OAUTH_REDIRECT_URI=aprendiendo-practice-mvp://oauth/callback
EXPO_PUBLIC_OAUTH_RESOURCE=https://practice.example.test
```

Use a development build for microphone, SQLite, secure storage, image selection, and document-file behavior. Expo web export or Expo Go is useful for layout checks but does not verify native recording reliability. The acceptance matrix and current evidence are in [`docs/recording-app/mobile-validation.md`](../../docs/recording-app/mobile-validation.md).

## Docker Compose

To start the practice API and web app together, from the repository root run:

```sh
docker compose -f compose.practice.yaml up --build
```

Open [http://localhost:8081](http://localhost:8081). The API is available at
`http://localhost:8082`; Gemini credentials are loaded into the API container from
the ignored `apps/practice/.env.mvp`, `apps/practice/.env.mcp`, or
`apps/practice/.env` files and are not included in the web image. Stop the stack
with `docker compose -f compose.practice.yaml down`.

For correlated API diagnostics, follow the API container logs while reproducing
a request:

```sh
docker compose -f compose.practice.yaml logs -f practice-api
```

Each request has an `mvp-N` request ID. The browser logs the same ID on an HTTP
error, and Gemini failures include the upstream status, `Retry-After` value,
provider error code/message, response size, and elapsed time. Audio, task text,
and API keys are not logged.

The root `compose.yaml` remains the production MCP/Caddy stack and is intentionally
separate from this local practice stack.

## Demo mode first

From the repository root, run the web app:

```sh
cd apps/practice
npm install
npm run web -- --port 8081
```

Open [http://localhost:8081](http://localhost:8081), leave the mode selector on **Demo**, and inspect the full feedback layout without microphone access or a key. Demo quotes and moments are fixture data, not evidence from the current take, and their replay actions stay disabled.

## Gemini mode

In a second terminal, from the repository root, create or edit one of the ignored
practice env files. The backend checks `.env.mvp`, `.env.mcp`, `.env`, then the
equivalent files under `apps/practice/`; existing shell environment variables win.
For this checkout, `apps/practice/.env` or `apps/practice/.env.mcp` is convenient:

```dotenv
GEMINI_API_KEY=put-your-local-key-here
GEMINI_MODEL=gemini-3.8-flash
MVP_WEB_ORIGIN=http://localhost:8081
```

Do not put this key in `EXPO_PUBLIC_*` variables or paste it into chat. Restart the
separate API after changing the file:

```sh
cargo run --bin practice_mvp
```

The manual browser API listens only on `127.0.0.1:8082`. The browser frontend’s nonsecret setting is optional because it defaults to that address; to override it, copy `.env.example` to an ignored `.env` and restart Expo:

```dotenv
EXPO_PUBLIC_MVP_API_URL=http://127.0.0.1:8082
```

Return to `http://localhost:8081`, choose **Gemini**, select a topic, and click **Record**. Permission is requested only at that point. Stop after at least 10 seconds, replay/seek the original take, then click **Get feedback**. A request is sent only by that button. The page shows the configured model, reported input/output token usage, an approximate API cost for supported Gemini Flash models, approximate model timestamps, the generated transcript, and a manual retry action. A timeout or provider error keeps the take available and does not automatically retry.

The web capture path prefers the browser codec reported by `expo-audio` (normally WebM/Opus); browsers that produce MP4/M4A are accepted when the bytes have an MP4 container. The app retains the actual MIME type and byte count and never relabels WebM bytes as WAV. If microphone capture is unavailable, the **Choose audio file (fallback)** control is explicitly labeled and is not a substitute for verifying microphone capture. Download is available from each local player on web.

## Checks

From the repository root:

```sh
cargo fmt --all -- --check
cargo test practice_mvp
cd apps/practice
npm run typecheck
npm run export:web
```

## Maestro E2E tests

The native app has offline-safe Maestro smoke flows for the practice modes,
demo feedback, settings, and empty history navigation. They do not start the
microphone or submit audio for cloud coaching. Install Maestro and run them
against a connected Android emulator or device with the native development
build installed:

```sh
cd apps/practice
npm run test:e2e
```

The flows live in [`./.maestro`](./.maestro). The Android package under test is
`com.timdumol.aprendiendo.practice`; build/install it first with
`npx expo run:android` or the repository's Android build script.

The real Gemini check requires a configured local key and a recording made explicitly for this experiment. Do not save private audio in the repository. A Demo screenshot is only a UI preview; it does not verify microphone capture or a model call.

Verified on 2026-09-08: `cargo fmt --all -- --check`, `cargo test --lib` (40 tests), `npm run typecheck`, `npm run export:web` (static `/`, `/history`, `/settings`, `/logs`, `/_sitemap`, and `+not-found` routes), the arm64 release APK build/install/launch workflow on a physical Android 16 device, and the live Pocket ID client/resource smoke checks. Native microphone capture, physical-device failure recovery, a real user-authenticated job round trip, and paid Gemini submissions remain unverified here; see `mobile-validation.md`.

## Limits and cost context

The browser MVP remains intentionally local and does not provide authentication, durable history, upload jobs, OAuth, TLS, or browser crash recovery. The native path has local durable history, an authenticated upload/job contract, bounded outbox state, a deterministic server worker, native AAC decoding for measurements, safe image validation, and PKCE sign-in wiring. Android build/install/launch is verified on one physical device; native recording, interruption recovery, device networking, store builds, and provider round trips remain unverified until the scenarios in `mobile-validation.md` are run. There is no transcript editor, numeric fluency score, exact acoustic measurement, or model-generated comparison. Timestamps are approximate model estimates and should be checked against the original audio.

The prior planning estimate was about three cents for nine Gemini audio minutes plus modest feedback/comparison. This prototype requests a faithful transcript and permits more output, so that is not an exact per-session quote; it makes no comparison call. The feedback response and server completion log report provider token usage when Gemini returns it, plus an approximate paid-tier cost for known Gemini Flash model IDs. Check the provider billing console for an invoice-level total.
