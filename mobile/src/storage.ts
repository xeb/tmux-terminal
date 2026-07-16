import AsyncStorage from '@react-native-async-storage/async-storage';

const SELECTED_TARGET_KEY = 'tmux-selected-target';
const HISTORY_KEY = 'tmux-command-history';
const HISTORY_MAX = 100;

export async function getSelectedTarget(): Promise<string | null> {
  return AsyncStorage.getItem(SELECTED_TARGET_KEY);
}

export async function saveSelectedTarget(target: string): Promise<void> {
  await AsyncStorage.setItem(SELECTED_TARGET_KEY, target);
}

export async function getHistory(): Promise<string[]> {
  try {
    const raw = await AsyncStorage.getItem(HISTORY_KEY);
    return raw ? (JSON.parse(raw) as string[]) : [];
  } catch {
    return [];
  }
}

export async function addToHistory(command: string): Promise<void> {
  if (!command.trim()) return;
  const history = await getHistory();
  if (history.length > 0 && history[history.length - 1] === command) return;
  history.push(command);
  const trimmed =
    history.length > HISTORY_MAX ? history.slice(-HISTORY_MAX) : history;
  await AsyncStorage.setItem(HISTORY_KEY, JSON.stringify(trimmed));
}
