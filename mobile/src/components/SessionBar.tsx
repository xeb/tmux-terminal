import React from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  ActivityIndicator,
} from 'react-native';
import { Picker } from '@react-native-picker/picker';
import { theme } from '../theme';
import type { TmuxWindow } from '../api';

interface Props {
  windows: TmuxWindow[];
  selectedTarget: string;
  onSelectTarget: (target: string) => void;
  onRefresh: () => void;
  isRefreshing: boolean;
}

export default function SessionBar({
  windows,
  selectedTarget,
  onSelectTarget,
  onRefresh,
  isRefreshing,
}: Props) {
  return (
    <View style={styles.container}>
      <Text style={styles.label}>TARGET:</Text>

      <View style={styles.pickerWrap}>
        <Picker
          selectedValue={selectedTarget}
          onValueChange={onSelectTarget}
          style={styles.picker}
          dropdownIconColor={theme.colors.text}
          mode="dropdown"
        >
          {windows.length === 0 ? (
            <Picker.Item label="Loading..." value="" color={theme.colors.textDim} />
          ) : (
            windows.map((w) => (
              <Picker.Item
                key={w.target}
                label={`${w.target}  ${w.name}`}
                value={w.target}
                color={theme.colors.text}
              />
            ))
          )}
        </Picker>
      </View>

      <TouchableOpacity
        style={styles.refreshBtn}
        onPress={onRefresh}
        disabled={isRefreshing}
        activeOpacity={0.6}
      >
        {isRefreshing ? (
          <ActivityIndicator size="small" color={theme.colors.text} />
        ) : (
          <Text style={styles.refreshText}>REFRESH</Text>
        )}
      </TouchableOpacity>

    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: theme.sp.sm,
    paddingVertical: theme.sp.xs,
    backgroundColor: theme.colors.bg,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
    gap: theme.sp.xs,
    minHeight: 44,
  },
  label: {
    fontFamily: theme.font,
    fontSize: theme.sz.xs,
    color: theme.colors.textDim,
    letterSpacing: 0.8,
    flexShrink: 0,
  },
  pickerWrap: {
    flex: 1,
    borderWidth: 1,
    borderColor: theme.colors.border,
    height: 34,
    justifyContent: 'center',
    overflow: 'hidden',
  },
  picker: {
    color: theme.colors.text,
    height: 34,
    marginTop: -6,
    marginBottom: -6,
  },
  refreshBtn: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    paddingHorizontal: theme.sp.md,
    paddingVertical: theme.sp.xs,
    height: 34,
    justifyContent: 'center',
    alignItems: 'center',
    flexShrink: 0,
    minWidth: 68,
  },
  refreshText: {
    fontFamily: theme.font,
    fontSize: 9,
    color: theme.colors.textDim,
    letterSpacing: 0.8,
  },
});
