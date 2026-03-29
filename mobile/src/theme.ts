import { Platform } from 'react-native';

// Exact match of the web app's CSS custom properties
export const theme = {
  colors: {
    bg: '#f8f8f8',           // --terminal-bg
    surface: '#ffffff',
    text: '#000000',         // --matrix-green (black in light mode)
    textDim: '#444444',      // --matrix-dim
    border: '#444444',
    borderLight: '#cccccc',
    btnBg: '#000000',
    btnText: '#ffffff',
    tabActiveBg: '#000000',
    tabActiveText: '#ffffff',
    tabBg: '#f8f8f8',
    tabText: '#000000',
    statusBg: '#000000',
    statusText: '#ffffff',
    errorBg: '#000000',
  },
  // IBM Plex Mono on web; closest native equivalents
  font: Platform.select({
    ios: 'Courier New',
    android: 'monospace',
    default: 'monospace',
  }) as string,
  sz: {
    xs: 10,
    sm: 12,
    md: 14,
    lg: 16,
    xl: 20,
  },
  sp: {
    xs: 4,
    sm: 6,
    md: 8,
    lg: 12,
    xl: 16,
  },
};
