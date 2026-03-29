import React from 'react';
import { StatusBar } from 'expo-status-bar';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { ErrorBoundary } from './src/ErrorBoundary';
import { theme } from './src/theme';
import AppTabs from './src/AppTabs';

export default function App() {
  return (
    <ErrorBoundary>
      <SafeAreaProvider>
        <StatusBar style="dark" backgroundColor={theme.colors.bg} />
        <AppTabs />
      </SafeAreaProvider>
    </ErrorBoundary>
  );
}
