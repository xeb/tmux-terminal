import React from 'react';
import { View, Text, ScrollView, TouchableOpacity, StyleSheet } from 'react-native';

interface State {
  error: Error | null;
}

export class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  State
> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  render() {
    if (!this.state.error) return this.props.children;
    const err = this.state.error;
    return (
      <View style={s.root}>
        <Text style={s.title}>CRASH</Text>
        <Text style={s.name}>{err.name}: {err.message}</Text>
        <ScrollView style={s.scroll}>
          <Text style={s.stack}>{err.stack}</Text>
        </ScrollView>
        <TouchableOpacity style={s.btn} onPress={() => this.setState({ error: null })}>
          <Text style={s.btnText}>RETRY</Text>
        </TouchableOpacity>
      </View>
    );
  }
}

const s = StyleSheet.create({
  root: { flex: 1, backgroundColor: '#f8f8f8', padding: 20, paddingTop: 60 },
  title: { fontFamily: 'Courier New', fontSize: 18, fontWeight: '700', marginBottom: 8 },
  name: { fontFamily: 'Courier New', fontSize: 12, marginBottom: 12, color: '#c00' },
  scroll: { flex: 1, borderWidth: 1, borderColor: '#444', padding: 8, marginBottom: 12 },
  stack: { fontFamily: 'Courier New', fontSize: 10, lineHeight: 16, color: '#333' },
  btn: { backgroundColor: '#000', padding: 14, alignItems: 'center' },
  btnText: { fontFamily: 'Courier New', color: '#fff', letterSpacing: 2 },
});
