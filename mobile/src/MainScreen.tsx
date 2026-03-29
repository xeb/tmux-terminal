import React, {
  useState,
  useEffect,
  useRef,
  useCallback,
} from 'react';
import {
  View,
  TouchableOpacity,
  Text,
  StyleSheet,
  KeyboardAvoidingView,
  Platform,
  Animated,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';

import { listWindows, capturePane, sendCommand, type TmuxWindow } from './api';
import {
  getSelectedTarget,
  saveSelectedTarget,
  addToHistory,
  getHistory,
} from './storage';
import { POLL_INTERVAL_MS } from './config';
import { theme } from './theme';
import { useVoice } from './useVoice';

import SessionBar from './components/SessionBar';
import TerminalOutput from './components/TerminalOutput';
import CommandInput from './components/CommandInput';
import WindowTabBar from './components/WindowTabBar';

export default function MainScreen() {
  const [windows, setWindows] = useState<TmuxWindow[]>([]);
  const [selectedTarget, setSelectedTarget] = useState('');
  const [terminalContent, setTerminalContent] = useState('Connecting...');
  const [command, setCommand] = useState('');
  const [isSending, setIsSending] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [statusMsg, setStatusMsg] = useState('');
  const [statusType, setStatusType] = useState<'ok' | 'err'>('ok');
  const [historyIdx, setHistoryIdx] = useState(-1);
  const [savedInput, setSavedInput] = useState('');

  const captureTimer = useRef<ReturnType<typeof setInterval> | null>(null);
  const lastContent = useRef('');
  const statusAnim = useRef(new Animated.Value(0)).current;

  // ─── Voice ────────────────────────────────────────────────────────────────
  const { isListening, toggle: voiceToggle } = useVoice(
    useCallback((transcript: string) => {
      setCommand((prev) => (prev ? `${prev} ${transcript}` : transcript));
    }, []),
  );

  // ─── Status toast ─────────────────────────────────────────────────────────
  const showStatus = useCallback(
    (msg: string, type: 'ok' | 'err' = 'ok') => {
      setStatusMsg(msg);
      setStatusType(type);
      Animated.sequence([
        Animated.timing(statusAnim, { toValue: 1, duration: 120, useNativeDriver: true }),
        Animated.delay(1800),
        Animated.timing(statusAnim, { toValue: 0, duration: 200, useNativeDriver: true }),
      ]).start();
    },
    [statusAnim],
  );

  // ─── Capture loop ─────────────────────────────────────────────────────────
  const startCapture = useCallback((target: string) => {
    if (captureTimer.current) clearInterval(captureTimer.current);
    const doCapture = async () => {
      if (!target) return;
      try {
        const content = await capturePane(target);
        if (content && content !== lastContent.current) {
          lastContent.current = content;
          setTerminalContent(content);
        }
      } catch {
        // silent
      }
    };
    doCapture();
    captureTimer.current = setInterval(doCapture, POLL_INTERVAL_MS);
  }, []);

  // ─── Load windows ─────────────────────────────────────────────────────────
  const loadWindows = useCallback(async () => {
    setIsRefreshing(true);
    try {
      const wins = await listWindows();
      setWindows(wins);
      if (wins.length > 0) {
        const saved = await getSelectedTarget();
        const found = saved && wins.find((w) => w.target === saved);
        const target = found ? saved! : wins[0].target;
        setSelectedTarget(target);
        lastContent.current = '';
        startCapture(target);
        showStatus(`${wins.length} WINDOWS`);
      }
    } catch {
      showStatus('CONNECTION FAILED', 'err');
    } finally {
      setIsRefreshing(false);
    }
  }, [startCapture, showStatus]);

  useEffect(() => {
    loadWindows();
    return () => {
      if (captureTimer.current) clearInterval(captureTimer.current);
    };
  }, []);  // eslint-disable-line react-hooks/exhaustive-deps

  // ─── Target / window selection ────────────────────────────────────────────
  const handleSelectTarget = useCallback(
    (target: string) => {
      if (!target) return;
      setSelectedTarget(target);
      saveSelectedTarget(target);
      lastContent.current = '';
      startCapture(target);
    },
    [startCapture],
  );

  const handleSelectWindow = useCallback(
    (target: string) => {
      handleSelectTarget(target);
      const win = windows.find((w) => w.target === target);
      showStatus(`→ ${win?.name ?? target}`);
    },
    [handleSelectTarget, windows, showStatus],
  );

  // ─── Send command ─────────────────────────────────────────────────────────
  const handleSend = useCallback(async () => {
    if (isSending) return;
    const cmd = command;
    const session = selectedTarget || '0';
    setIsSending(true);
    try {
      const result = await sendCommand(cmd, session);
      if (result.success) {
        showStatus(cmd.trim() ? 'SENT' : 'ENTER');
        if (cmd.trim()) await addToHistory(cmd.trim());
        setCommand('');
        setHistoryIdx(-1);
        setSavedInput('');
        setTimeout(async () => {
          try {
            const content = await capturePane(session);
            if (content) { lastContent.current = content; setTerminalContent(content); }
          } catch {}
        }, 120);
      } else {
        showStatus(result.error ?? 'FAILED', 'err');
      }
    } catch {
      showStatus('CONNECTION ERROR', 'err');
    } finally {
      setIsSending(false);
    }
  }, [command, selectedTarget, isSending, showStatus]);

  // ─── History navigation ───────────────────────────────────────────────────
  const handleHistoryUp = useCallback(async () => {
    const history = await getHistory();
    if (history.length === 0) return;
    if (historyIdx === -1) setSavedInput(command);
    const newIdx = Math.min(historyIdx + 1, history.length - 1);
    setHistoryIdx(newIdx);
    setCommand(history[history.length - 1 - newIdx]);
  }, [historyIdx, command]);

  const handleHistoryDown = useCallback(async () => {
    if (historyIdx === -1) return;
    const newIdx = historyIdx - 1;
    if (newIdx === -1) {
      setHistoryIdx(-1);
      setCommand(savedInput);
    } else {
      const history = await getHistory();
      setHistoryIdx(newIdx);
      setCommand(history[history.length - 1 - newIdx]);
    }
  }, [historyIdx, savedInput]);

  // ─── Render ───────────────────────────────────────────────────────────────
  return (
    <SafeAreaView style={styles.safe} edges={['top']}>
      <KeyboardAvoidingView
        style={styles.root}
        behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
        keyboardVerticalOffset={0}
      >
        <SessionBar
          windows={windows}
          selectedTarget={selectedTarget}
          onSelectTarget={handleSelectTarget}
          onRefresh={loadWindows}
          isRefreshing={isRefreshing}
        />

        <Animated.View
          style={[styles.status, statusType === 'err' && styles.statusErr, { opacity: statusAnim }]}
          pointerEvents="none"
        >
          <Text style={styles.statusText}>{statusMsg}</Text>
        </Animated.View>

        <TerminalOutput content={terminalContent} />

        <CommandInput
          value={command}
          onChange={(t) => { setCommand(t); setHistoryIdx(-1); }}
          onVoiceToggle={voiceToggle}
          isListening={isListening}
          onHistoryUp={handleHistoryUp}
          onHistoryDown={handleHistoryDown}
        />

        <TouchableOpacity
          style={[styles.execBtn, isSending && styles.execBtnDisabled]}
          onPress={handleSend}
          disabled={isSending}
          activeOpacity={0.8}
        >
          <Text style={styles.execText}>{isSending ? 'SENDING...' : 'EXECUTE'}</Text>
        </TouchableOpacity>

        <WindowTabBar
          windows={windows}
          selectedTarget={selectedTarget}
          onSelect={handleSelectWindow}
        />
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: theme.colors.bg },
  root: { flex: 1, backgroundColor: theme.colors.bg },
  status: {
    position: 'absolute',
    top: 44,
    left: 0,
    right: 0,
    backgroundColor: theme.colors.statusBg,
    paddingHorizontal: theme.sp.lg,
    paddingVertical: theme.sp.xs,
    zIndex: 100,
  },
  statusErr: { backgroundColor: '#333333' },
  statusText: {
    fontFamily: theme.font,
    fontSize: theme.sz.xs,
    color: theme.colors.statusText,
    letterSpacing: 1,
  },
  execBtn: {
    backgroundColor: theme.colors.btnBg,
    marginHorizontal: theme.sp.sm,
    marginBottom: theme.sp.xs,
    paddingVertical: 18,
    alignItems: 'center',
    justifyContent: 'center',
  },
  execBtnDisabled: { opacity: 0.45 },
  execText: {
    fontFamily: theme.font,
    fontSize: theme.sz.lg,
    fontWeight: '700',
    color: theme.colors.btnText,
    letterSpacing: 4,
  },
});
