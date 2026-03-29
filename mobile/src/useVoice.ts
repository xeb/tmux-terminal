import { useState, useCallback } from 'react';

// expo-speech-recognition removed — voice input disabled while we stabilise native builds.
// The mic button will be hidden (available: false).

export function useVoice(_onTranscript: (text: string) => void) {
  const [isListening] = useState(false);

  const toggle = useCallback(async () => {
    // noop
  }, []);

  return { isListening, toggle, available: false };
}
