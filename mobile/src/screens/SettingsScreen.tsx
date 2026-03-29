import React from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  StyleSheet,
  ScrollView,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { theme } from '../theme';
import { BUILD_INFO } from '../buildInfo';
import { SERVER_URL } from '../config';

interface Props {
  onReportBug: () => void;
}

export default function SettingsScreen({ onReportBug }: Props) {
  return (
    <SafeAreaView style={s.safe} edges={['top', 'left', 'right']}>
      <ScrollView style={s.scroll} contentContainerStyle={s.content}>
        <Text style={s.header}>SETTINGS</Text>

        <View style={s.section}>
          <Text style={s.sectionLabel}>BUILD INFO</Text>
          <View style={s.card}>
            <Row label="Version" value={BUILD_INFO.version} />
            <Row label="Built" value={BUILD_INFO.buildDate.replace('T', ' ').replace('Z', ' UTC')} />
            <Row label="Commit" value={BUILD_INFO.commitHash} />
            <Row label="Server" value={SERVER_URL.replace('http://', '')} />
          </View>
        </View>

        <View style={s.section}>
          <TouchableOpacity style={s.bugButton} onPress={onReportBug} activeOpacity={0.75}>
            <Text style={s.bugButtonText}>[ REPORT A BUG → ]</Text>
          </TouchableOpacity>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <View style={s.row}>
      <Text style={s.rowLabel}>{label}:</Text>
      <Text style={s.rowValue} numberOfLines={1} adjustsFontSizeToFit>
        {value}
      </Text>
    </View>
  );
}

const s = StyleSheet.create({
  safe: {
    flex: 1,
    backgroundColor: theme.colors.bg,
  },
  scroll: {
    flex: 1,
  },
  content: {
    padding: theme.sp.xl,
  },
  header: {
    fontFamily: theme.font,
    fontSize: theme.sz.xl,
    fontWeight: 'bold',
    color: theme.colors.text,
    letterSpacing: 2,
    marginBottom: theme.sp.xl,
    borderBottomWidth: 2,
    borderBottomColor: theme.colors.text,
    paddingBottom: theme.sp.md,
  },
  section: {
    marginBottom: theme.sp.xl,
  },
  sectionLabel: {
    fontFamily: theme.font,
    fontSize: theme.sz.xs,
    fontWeight: 'bold',
    color: theme.colors.textDim,
    letterSpacing: 2,
    marginBottom: theme.sp.sm,
  },
  card: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface,
  },
  row: {
    flexDirection: 'row',
    paddingHorizontal: theme.sp.md,
    paddingVertical: theme.sp.sm,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.borderLight,
  },
  rowLabel: {
    fontFamily: theme.font,
    fontSize: theme.sz.sm,
    color: theme.colors.textDim,
    width: 70,
  },
  rowValue: {
    fontFamily: theme.font,
    fontSize: theme.sz.sm,
    color: theme.colors.text,
    flex: 1,
  },
  bugButton: {
    borderWidth: 1,
    borderColor: theme.colors.text,
    padding: theme.sp.lg,
    alignItems: 'center',
  },
  bugButtonText: {
    fontFamily: theme.font,
    fontSize: theme.sz.md,
    fontWeight: 'bold',
    color: theme.colors.text,
    letterSpacing: 1,
  },
});
