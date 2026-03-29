import React, { useState } from 'react';
import { View, Text, TouchableOpacity, StyleSheet } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { theme } from './theme';
import MainScreen from './MainScreen';
import SettingsScreen from './screens/SettingsScreen';

// Lazy-load BugReportScreen so expo-image-picker native module is only
// accessed when the user actually navigates to that screen.
function getBugReportScreen(): React.ComponentType<{ onBack: () => void }> {
  return require('./screens/BugReportScreen').default;
}

type Tab = 'terminal' | 'settings';
type SettingsStack = 'main' | 'bugreport';

export default function AppTabs() {
  const [activeTab, setActiveTab] = useState<Tab>('terminal');
  const [settingsStack, setSettingsStack] = useState<SettingsStack>('main');

  function switchTab(tab: Tab) {
    setActiveTab(tab);
    if (tab === 'settings') setSettingsStack('main');
  }

  return (
    <View style={s.container}>
      {/* Content area */}
      <View style={s.content}>
        {activeTab === 'terminal' ? (
          <MainScreen />
        ) : settingsStack === 'bugreport' ? (
          (() => { const BugReportScreen = getBugReportScreen(); return <BugReportScreen onBack={() => setSettingsStack('main')} />; })()
        ) : (
          <SettingsScreen onReportBug={() => setSettingsStack('bugreport')} />
        )}
      </View>

      {/* Bottom tab bar */}
      <View style={s.tabBar}>
        <TabButton
          label="TERMINAL"
          icon="terminal-outline"
          active={activeTab === 'terminal'}
          onPress={() => switchTab('terminal')}
        />
        <TabButton
          label="SETTINGS"
          icon="settings-outline"
          active={activeTab === 'settings'}
          onPress={() => switchTab('settings')}
        />
      </View>
    </View>
  );
}

function TabButton({
  label,
  icon,
  active,
  onPress,
}: {
  label: string;
  icon: keyof typeof Ionicons.glyphMap;
  active: boolean;
  onPress: () => void;
}) {
  return (
    <TouchableOpacity
      style={[s.tab, active && s.tabActive]}
      onPress={onPress}
      activeOpacity={0.8}
    >
      <Ionicons
        name={icon}
        size={18}
        color={active ? theme.colors.tabActiveText : theme.colors.tabText}
      />
      <Text style={[s.tabLabel, active && s.tabLabelActive]}>{label}</Text>
    </TouchableOpacity>
  );
}

const s = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: theme.colors.bg,
  },
  content: {
    flex: 1,
  },
  tabBar: {
    flexDirection: 'row',
    borderTopWidth: 1,
    borderTopColor: theme.colors.border,
    backgroundColor: theme.colors.bg,
  },
  tab: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 6,
    paddingVertical: 10,
    borderRightWidth: 1,
    borderRightColor: theme.colors.border,
    backgroundColor: theme.colors.tabBg,
  },
  tabActive: {
    backgroundColor: theme.colors.tabActiveBg,
  },
  tabLabel: {
    fontFamily: theme.font,
    fontSize: theme.sz.xs,
    fontWeight: 'bold',
    color: theme.colors.tabText,
    letterSpacing: 1,
  },
  tabLabelActive: {
    color: theme.colors.tabActiveText,
  },
});
