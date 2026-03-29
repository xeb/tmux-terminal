import React, { useRef, useEffect } from 'react';
import {
  ScrollView,
  TouchableOpacity,
  Text,
  StyleSheet,
  View,
} from 'react-native';
import { theme } from '../theme';
import type { TmuxWindow } from '../api';

interface Props {
  windows: TmuxWindow[];
  selectedTarget: string;
  onSelect: (target: string) => void;
}

export default function WindowTabBar({ windows, selectedTarget, onSelect }: Props) {
  const scrollRef = useRef<ScrollView>(null);

  // Scroll to show the active tab when selection changes
  useEffect(() => {
    const idx = windows.findIndex((w) => w.target === selectedTarget);
    if (idx > 0 && scrollRef.current) {
      // Approximate scroll: each tab ~80px wide
      scrollRef.current.scrollTo({ x: Math.max(0, (idx - 1) * 80), animated: true });
    }
  }, [selectedTarget, windows]);

  if (windows.length === 0) return null;

  return (
    <View style={styles.container}>
      <ScrollView
        ref={scrollRef}
        horizontal
        showsHorizontalScrollIndicator={false}
        keyboardShouldPersistTaps="handled"
        contentContainerStyle={styles.scrollContent}
      >
        {windows.map((win) => {
          const active = win.target === selectedTarget;
          return (
            <TouchableOpacity
              key={win.target}
              style={[styles.tab, active && styles.tabActive]}
              onPress={() => onSelect(win.target)}
              activeOpacity={0.7}
            >
              <Text
                style={[styles.tabName, active && styles.tabNameActive]}
                numberOfLines={1}
              >
                {win.name}
              </Text>
              <Text
                style={[styles.tabTarget, active && styles.tabTargetActive]}
              >
                {win.target}
              </Text>
            </TouchableOpacity>
          );
        })}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    borderTopWidth: 1,
    borderTopColor: theme.colors.border,
    backgroundColor: theme.colors.bg,
    height: 50,
  },
  scrollContent: {
    flexDirection: 'row',
    alignItems: 'stretch',
  },
  tab: {
    paddingHorizontal: 12,
    paddingVertical: 4,
    borderRightWidth: 1,
    borderRightColor: theme.colors.borderLight,
    justifyContent: 'center',
    alignItems: 'center',
    minWidth: 64,
    height: 49,
    backgroundColor: theme.colors.tabBg,
    gap: 1,
  },
  tabActive: {
    backgroundColor: theme.colors.tabActiveBg,
    borderRightColor: theme.colors.border,
  },
  tabName: {
    fontFamily: theme.font,
    fontSize: 11,
    color: theme.colors.tabText,
    letterSpacing: 0.3,
    maxWidth: 100,
  },
  tabNameActive: {
    color: theme.colors.tabActiveText,
    fontWeight: '700',
  },
  tabTarget: {
    fontFamily: theme.font,
    fontSize: 9,
    color: theme.colors.textDim,
    letterSpacing: 0.2,
  },
  tabTargetActive: {
    color: 'rgba(255,255,255,0.6)',
  },
});
