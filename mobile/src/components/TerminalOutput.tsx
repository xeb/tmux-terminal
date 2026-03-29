import React, { useRef, useEffect, useCallback } from 'react';
import {
  ScrollView,
  Text,
  StyleSheet,
  View,
  NativeScrollEvent,
  NativeSyntheticEvent,
} from 'react-native';
import { theme } from '../theme';

// Strip ANSI escape sequences so the raw tmux capture looks clean
function stripAnsi(text: string): string {
  return text
    .replace(/\x1b\[[0-9;]*[a-zA-Z]/g, '')
    .replace(/\x1b\][^\x07]*\x07/g, '')
    .replace(/\x1b[()][0-9A-Za-z]/g, '')
    .replace(/\x1b./g, '');
}

interface Props {
  content: string;
}

export default function TerminalOutput({ content }: Props) {
  const scrollRef = useRef<ScrollView>(null);
  const atBottomRef = useRef(true);

  const handleScroll = useCallback(
    (e: NativeSyntheticEvent<NativeScrollEvent>) => {
      const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
      atBottomRef.current =
        contentSize.height - (contentOffset.y + layoutMeasurement.height) < 60;
    },
    [],
  );

  useEffect(() => {
    if (atBottomRef.current) {
      // Use a short delay so layout completes before scrolling
      const id = setTimeout(
        () => scrollRef.current?.scrollToEnd({ animated: false }),
        16,
      );
      return () => clearTimeout(id);
    }
  }, [content]);

  const displayText = stripAnsi(content);

  return (
    <View style={styles.frame}>
      <ScrollView
        ref={scrollRef}
        style={styles.scroll}
        onScroll={handleScroll}
        scrollEventThrottle={100}
        keyboardShouldPersistTaps="handled"
        showsVerticalScrollIndicator
        indicatorStyle="black"
      >
        <Text style={styles.text} selectable>
          {displayText}
        </Text>
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  frame: {
    flex: 1,
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.bg,
    marginHorizontal: theme.sp.sm,
    marginTop: theme.sp.xs,
    marginBottom: theme.sp.xs,
  },
  scroll: {
    flex: 1,
  },
  text: {
    fontFamily: theme.font,
    fontSize: theme.sz.sm,
    color: theme.colors.text,
    lineHeight: 18,
    padding: theme.sp.sm,
    // Ensure long lines wrap rather than scroll horizontally
    flexWrap: 'wrap',
  },
});
