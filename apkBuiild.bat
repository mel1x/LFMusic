pnpm tauri android build --apk
apksigner sign --ks release-key.jks --out app-release.apk src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk
pause