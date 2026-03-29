import React, { useRef } from 'react';
import {
  View,
  TextInput,
  TouchableOpacity,
  StyleSheet,
  Text,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { theme } from '../theme';

interface Props {
  value: string;
  onChange: (text: string) => void;
  onVoiceToggle: () => void;
  isListening: boolean;
  onHistoryUp: () => void;
  onHistoryDown: () => void;
}

export default function CommandInput({
  value,
  onChange,
  onVoiceToggle,
  isListening,
  onHistoryUp,
  onHistoryDown,
}: Props) {
  const inputRef = useRef<TextInput>(null);

  return (
    <View style={styles.container}>
      {/* History navigation buttons */}
      <View style={styles.historyBtns}>
        <TouchableOpacity
          style={styles.historyBtn}
          onPress={onHistoryUp}
          activeOpacity={0.6}
        >
          <Text style={styles.historyBtnText}>▲</Text>
        </TouchableOpacity>
        <TouchableOpacity
          style={styles.historyBtn}
          onPress={onHistoryDown}
          activeOpacity={0.6}
        >
          <Text style={styles.historyBtnText}>▼</Text>
        </TouchableOpacity>
      </View>

      {/* Command text input */}
      <TextInput
        ref={inputRef}
        style={styles.input}
        value={value}
        onChangeText={onChange}
        multiline
        numberOfLines={4}
        placeholder="Enter command..."
        placeholderTextColor={theme.colors.textDim}
        autoCapitalize="none"
        autoCorrect={false}
        spellCheck={false}
        textAlignVertical="top"
        returnKeyType="default"
        blurOnSubmit={false}
      />

      {/* Voice input button */}
      <TouchableOpacity
        style={[styles.micBtn, isListening && styles.micBtnActive]}
        onPress={onVoiceToggle}
        activeOpacity={0.7}
      >
        <Ionicons
          name={isListening ? 'mic' : 'mic-outline'}
          size={20}
          color={isListening ? theme.colors.btnText : theme.colors.text}
        />
        {isListening && <Text style={styles.micLabel}>●</Text>}
      </TouchableOpacity>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flexDirection: 'row',
    alignItems: 'flex-start',
    marginHorizontal: theme.sp.sm,
    marginBottom: theme.sp.xs,
    gap: theme.sp.xs,
  },
  historyBtns: {
    gap: 2,
    paddingTop: 2,
  },
  historyBtn: {
    width: 28,
    height: 28,
    borderWidth: 1,
    borderColor: theme.colors.border,
    justifyContent: 'center',
    alignItems: 'center',
    backgroundColor: theme.colors.bg,
  },
  historyBtnText: {
    fontFamily: theme.font,
    fontSize: 10,
    color: theme.colors.textDim,
  },
  input: {
    flex: 1,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.bg,
    fontFamily: theme.font,
    fontSize: theme.sz.md,
    color: theme.colors.text,
    padding: theme.sp.md,
    minHeight: 90,
    textAlignVertical: 'top',
    // Prevent iOS from adding extra styling
    // @ts-ignore
    WebkitAppearance: 'none',
  },
  micBtn: {
    width: 44,
    height: 44,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: 22,
    justifyContent: 'center',
    alignItems: 'center',
    backgroundColor: theme.colors.bg,
    gap: 2,
    marginTop: 2,
  },
  micBtnActive: {
    backgroundColor: theme.colors.btnBg,
    borderColor: theme.colors.btnBg,
  },
  micLabel: {
    fontSize: 6,
    color: '#ff0000',
    lineHeight: 6,
  },
});
