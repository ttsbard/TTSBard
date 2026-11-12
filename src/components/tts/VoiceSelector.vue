<script setup lang="ts">
import { t } from '../../i18n';

interface Props {
  voices: string[];
  selectedVoiceId: string;
  loading?: boolean;
  label?: string;
}

interface Emits {
  (e: 'voice-change', voice: string): void;
  (e: 'refresh'): void;
}

const props = withDefaults(defineProps<Props>(), {
  loading: false,
  label: '',
});

const emit = defineEmits<Emits>();

function handleChange(event: Event) {
  const target = event.target as HTMLSelectElement;
  emit('voice-change', target.value);
}
</script>

<template>
  <div class="voice-selector">
    <label class="ui-label">{{ label || t('tts.voice') }}</label>
    <div class="voice-select-wrapper">
      <select
        :value="selectedVoiceId"
        @change="handleChange"
        :disabled="loading"
        class="ui-select voice-select"
      >
        <option v-for="voice in voices" :key="voice" :value="voice">
          {{ voice }}
        </option>
      </select>
    </div>
  </div>
</template>

<style scoped>
.voice-selector {
  display: flex;
  align-items: center;
  gap: var(--ui-row-label-gap-side);
  flex-wrap: wrap;
}

.voice-selector .ui-label {
  min-width: 60px;
}

.voice-select-wrapper {
  flex: 0 1 auto;
  min-width: 100px;
}

.voice-select {
  width: 100%;
}
</style>
