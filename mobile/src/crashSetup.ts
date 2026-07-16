/**
 * Must be imported as the VERY FIRST import in App.tsx.
 * Sets up global JS error handler before any other module loads.
 * Saves crash info to AsyncStorage so next launch can report it.
 */
import { ErrorUtils } from 'react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';

const CRASH_KEY = '@tmux_last_crash';
const SERVER = 'http://100.68.192.63:5533';

function postToServer(payload: object) {
  fetch(`${SERVER}/api/client-error`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  }).catch(() => {});
}

// Set up the global handler immediately at module load time
const originalHandler = ErrorUtils.getGlobalHandler();
ErrorUtils.setGlobalHandler((error: Error, isFatal: boolean) => {
  const report = {
    error: error?.message ?? String(error),
    stack: error?.stack ?? '(no stack)',
    context: `global_js_handler`,
    url: `fatal=${isFatal} ts=${Date.now()}`,
  };

  // Post immediately (fire and forget)
  postToServer(report);

  // Also save to AsyncStorage for display on next launch
  AsyncStorage.setItem(CRASH_KEY, JSON.stringify(report)).catch(() => {});

  // Call the original handler (re-throws in development, crashes in production)
  originalHandler(error, isFatal);
});

/** Call on app startup. Returns any crash info from the previous launch, then clears it. */
export async function popPreviousCrash(): Promise<string | null> {
  try {
    const raw = await AsyncStorage.getItem(CRASH_KEY);
    if (!raw) return null;
    await AsyncStorage.removeItem(CRASH_KEY);
    // Post to server now that network is up
    try {
      const parsed = JSON.parse(raw);
      postToServer({ ...parsed, context: 'previous_launch_crash' });
    } catch {}
    return raw;
  } catch {
    return null;
  }
}
