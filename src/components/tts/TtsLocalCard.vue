<script setup lang="ts">
import { ref, watch } from 'vue';
import { HardDrive } from 'lucide-vue-next';
import ProviderCard from '../shared/ProviderCard.vue';
import { t } from '../../i18n';

interface Props {
  active?: boolean;
  expanded?: boolean;
  url?: string;
}

interface Emits {
  (e: 'select'): void;
  (e: 'toggle'): void;
  (e: 'save', url: string): void;
}

const props = withDefaults(defineProps<Props>(), {
  active: false,
  expanded: false,
  url: 'http://127.0.0.1:8124',
});

const emit = defineEmits<Emits>();

const inputUrl = ref(props.url);

watch(() => props.url, (newUrl) => {
  inputUrl.value = newUrl;
});

function handleUrlKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter') {
    handleSave();
  }
}

function handleSave() {
  emit('save', inputUrl.value);
}
</script>

<template>
  <ProviderCard
    :title="t('tts.local.title')"
    :icon="HardDrive"
    :active="active"
    :expanded="expanded"
    @select="$emit('select')"
    @toggle="$emit('toggle')"
  >
    <div class="card-content-inner">
      <div class="card-subtitle">{{ t('tts.local.desc') }}</div>
      <div class="setting-group">
        <div class="local-url-row">
          <label class="ui-label">URL</label>
          <input
            v-model="inputUrl"
            @keydown="handleUrlKeydown"
            type="text"
            placeholder="http://127.0.0.1:8124"
            class="ui-input local-url-input"
          />
          <button @click="handleSave" class="ui-button ui-button--primary save-url-button">{{ t('common.save') }}</button>
        </div>
      </div>
    </div>
  </ProviderCard>
</template>

<style scoped>
.card-content-inner {
  padding-top: 8px;
}

.card-subtitle {
  font-size: var(--ui-text-size-hint);
  font-weight: var(--ui-text-weight-hint);
  color: var(--color-text-secondary);
  margin-bottom: 16px;
}

.setting-group {
  margin-bottom: var(--ui-row-gap);
}

.setting-group:last-child {
  margin-bottom: 0;
}

/* Local URL row */
.local-url-row {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  margin-bottom: 8px;
}

.local-url-row .ui-label {
  min-width: 60px;
}

.local-url-input {
  flex: 1;
  min-width: 200px;
}

.save-url-button {
  white-space: nowrap;
  flex-shrink: 0;
}
</style>
