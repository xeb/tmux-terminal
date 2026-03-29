import React, { useState } from 'react';
import {
  View,
  Text,
  TextInput,
  TouchableOpacity,
  StyleSheet,
  ScrollView,
  Image,
  Alert,
  ActivityIndicator,
  Platform,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as ImagePicker from 'expo-image-picker';
import { theme } from '../theme';
import { BUILD_INFO } from '../buildInfo';
import { submitBugReport } from '../api';

interface Props {
  onBack: () => void;
}

interface Screenshot {
  uri: string;
  base64: string;
  name: string;
}

export default function BugReportScreen({ onBack }: Props) {
  const [screenshots, setScreenshots] = useState<Screenshot[]>([]);
  const [description, setDescription] = useState('');
  const [status, setStatus] = useState<'idle' | 'submitting' | 'done' | 'error'>('idle');
  const [errorMsg, setErrorMsg] = useState('');

  const deviceName = Platform.OS === 'ios' ? 'iPhone' : 'Android Device';
  const osVersion = `${Platform.OS === 'ios' ? 'iOS' : 'Android'} ${Platform.Version}`;

  async function pickScreenshot() {
    if (screenshots.length >= 5) {
      Alert.alert('Max 5 screenshots');
      return;
    }
    const { status: permStatus } = await ImagePicker.requestMediaLibraryPermissionsAsync();
    if (permStatus !== 'granted') {
      Alert.alert('Permission required', 'Please allow photo library access to add screenshots.');
      return;
    }
    const result = await ImagePicker.launchImageLibraryAsync({
      mediaTypes: ['images'],
      quality: 0.8,
      base64: true,
    });
    if (!result.canceled && result.assets[0]) {
      const asset = result.assets[0];
      const base64 = asset.base64 ?? '';
      const name = `screenshot_${screenshots.length + 1}.jpg`;
      setScreenshots(prev => [...prev, { uri: asset.uri, base64, name }]);
    }
  }

  function removeScreenshot(index: number) {
    setScreenshots(prev => prev.filter((_, i) => i !== index));
  }

  async function handleSubmit() {
    setStatus('submitting');
    setErrorMsg('');
    try {
      const result = await submitBugReport({
        description: description.trim() || undefined,
        app_version: BUILD_INFO.version,
        build_date: BUILD_INFO.buildDate,
        device: deviceName,
        os: osVersion,
        screenshots: screenshots.map(s => ({ name: s.name, data: s.base64 })),
      });

      if (result.success) {
        setStatus('done');
        setTimeout(() => onBack(), 1500);
      } else {
        setStatus('error');
        setErrorMsg(result.error ?? 'Submission failed');
      }
    } catch (e: any) {
      setStatus('error');
      setErrorMsg(e?.message ?? 'Network error');
    }
  }

  if (status === 'done') {
    return (
      <SafeAreaView style={s.safe} edges={['top', 'left', 'right']}>
        <View style={s.doneContainer}>
          <Text style={s.doneText}>REPORT SUBMITTED</Text>
          <Text style={s.doneSubtext}>Returning to settings...</Text>
        </View>
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView style={s.safe} edges={['top', 'left', 'right']}>
      <View style={s.topBar}>
        <TouchableOpacity onPress={onBack} style={s.backBtn}>
          <Text style={s.backText}>← BACK</Text>
        </TouchableOpacity>
        <Text style={s.topTitle}>REPORT A BUG</Text>
        <View style={s.backBtn} />
      </View>

      <ScrollView style={s.scroll} contentContainerStyle={s.content} keyboardShouldPersistTaps="handled">
        {/* Screenshots */}
        <Text style={s.sectionLabel}>SCREENSHOTS  (tap + to add, max 5)</Text>
        <View style={s.screenshotsRow}>
          {screenshots.map((shot, i) => (
            <TouchableOpacity key={i} onPress={() => removeScreenshot(i)} style={s.thumbContainer}>
              <Image source={{ uri: shot.uri }} style={s.thumb} />
              <View style={s.thumbRemove}>
                <Text style={s.thumbRemoveText}>✕</Text>
              </View>
            </TouchableOpacity>
          ))}
          {screenshots.length < 5 && (
            <TouchableOpacity style={s.addThumb} onPress={pickScreenshot}>
              <Text style={s.addThumbText}>+</Text>
            </TouchableOpacity>
          )}
        </View>

        {/* Description */}
        <Text style={s.sectionLabel}>DESCRIPTION</Text>
        <TextInput
          style={s.descInput}
          placeholder="What went wrong?"
          placeholderTextColor={theme.colors.borderLight}
          multiline
          numberOfLines={5}
          value={description}
          onChangeText={setDescription}
          textAlignVertical="top"
        />

        {/* Meta info */}
        <Text style={s.sectionLabel}>WILL ALSO INCLUDE:</Text>
        <View style={s.card}>
          <MetaRow label="App version" value={BUILD_INFO.version} />
          <MetaRow label="Built" value={BUILD_INFO.buildDate.replace('T', ' ').replace('Z', ' UTC')} />
          <MetaRow label="Device" value={deviceName} />
          <MetaRow label="OS" value={osVersion} />
        </View>

        {status === 'error' && (
          <Text style={s.errorText}>ERROR: {errorMsg}</Text>
        )}

        {/* Submit */}
        <TouchableOpacity
          style={[s.submitBtn, status === 'submitting' && s.submitBtnDisabled]}
          onPress={handleSubmit}
          disabled={status === 'submitting'}
          activeOpacity={0.8}
        >
          {status === 'submitting' ? (
            <ActivityIndicator color={theme.colors.btnText} />
          ) : (
            <Text style={s.submitText}>SUBMIT REPORT</Text>
          )}
        </TouchableOpacity>
      </ScrollView>
    </SafeAreaView>
  );
}

function MetaRow({ label, value }: { label: string; value: string }) {
  return (
    <View style={s.metaRow}>
      <Text style={s.metaLabel}>{label}:</Text>
      <Text style={s.metaValue}>{value}</Text>
    </View>
  );
}

const THUMB_SIZE = 64;

const s = StyleSheet.create({
  safe: {
    flex: 1,
    backgroundColor: theme.colors.bg,
  },
  topBar: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingHorizontal: theme.sp.md,
    paddingVertical: theme.sp.sm,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.border,
  },
  backBtn: {
    width: 80,
  },
  backText: {
    fontFamily: theme.font,
    fontSize: theme.sz.sm,
    color: theme.colors.text,
    fontWeight: 'bold',
  },
  topTitle: {
    fontFamily: theme.font,
    fontSize: theme.sz.md,
    fontWeight: 'bold',
    color: theme.colors.text,
    letterSpacing: 1,
  },
  scroll: {
    flex: 1,
  },
  content: {
    padding: theme.sp.xl,
    gap: theme.sp.md,
  },
  sectionLabel: {
    fontFamily: theme.font,
    fontSize: theme.sz.xs,
    fontWeight: 'bold',
    color: theme.colors.textDim,
    letterSpacing: 2,
    marginTop: theme.sp.md,
  },
  screenshotsRow: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: theme.sp.sm,
    marginTop: theme.sp.xs,
  },
  thumbContainer: {
    width: THUMB_SIZE,
    height: THUMB_SIZE,
    borderWidth: 1,
    borderColor: theme.colors.border,
    position: 'relative',
  },
  thumb: {
    width: '100%',
    height: '100%',
  },
  thumbRemove: {
    position: 'absolute',
    top: 2,
    right: 2,
    backgroundColor: theme.colors.text,
    width: 16,
    height: 16,
    alignItems: 'center',
    justifyContent: 'center',
  },
  thumbRemoveText: {
    fontFamily: theme.font,
    fontSize: 9,
    color: theme.colors.btnText,
    fontWeight: 'bold',
  },
  addThumb: {
    width: THUMB_SIZE,
    height: THUMB_SIZE,
    borderWidth: 1,
    borderColor: theme.colors.border,
    alignItems: 'center',
    justifyContent: 'center',
    borderStyle: 'dashed',
  },
  addThumbText: {
    fontFamily: theme.font,
    fontSize: 24,
    color: theme.colors.textDim,
  },
  descInput: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    padding: theme.sp.md,
    fontFamily: theme.font,
    fontSize: theme.sz.sm,
    color: theme.colors.text,
    backgroundColor: theme.colors.surface,
    minHeight: 100,
    marginTop: theme.sp.xs,
  },
  card: {
    borderWidth: 1,
    borderColor: theme.colors.border,
    backgroundColor: theme.colors.surface,
    marginTop: theme.sp.xs,
  },
  metaRow: {
    flexDirection: 'row',
    paddingHorizontal: theme.sp.md,
    paddingVertical: theme.sp.sm,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.borderLight,
  },
  metaLabel: {
    fontFamily: theme.font,
    fontSize: theme.sz.sm,
    color: theme.colors.textDim,
    width: 90,
  },
  metaValue: {
    fontFamily: theme.font,
    fontSize: theme.sz.sm,
    color: theme.colors.text,
    flex: 1,
  },
  errorText: {
    fontFamily: theme.font,
    fontSize: theme.sz.sm,
    color: theme.colors.btnText,
    backgroundColor: theme.colors.errorBg,
    padding: theme.sp.md,
    marginTop: theme.sp.sm,
  },
  submitBtn: {
    backgroundColor: theme.colors.btnBg,
    padding: theme.sp.lg,
    alignItems: 'center',
    marginTop: theme.sp.xl,
    marginBottom: theme.sp.xl,
  },
  submitBtnDisabled: {
    opacity: 0.6,
  },
  submitText: {
    fontFamily: theme.font,
    fontSize: theme.sz.lg,
    fontWeight: 'bold',
    color: theme.colors.btnText,
    letterSpacing: 2,
  },
  doneContainer: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    gap: theme.sp.md,
  },
  doneText: {
    fontFamily: theme.font,
    fontSize: theme.sz.xl,
    fontWeight: 'bold',
    color: theme.colors.text,
    letterSpacing: 2,
  },
  doneSubtext: {
    fontFamily: theme.font,
    fontSize: theme.sz.sm,
    color: theme.colors.textDim,
  },
});
