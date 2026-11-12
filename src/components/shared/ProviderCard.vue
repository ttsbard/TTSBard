<script setup lang="ts">
import { computed } from 'vue';
import type { Component } from 'vue';
import { restoreProviderRadioState } from './providerCardState';

interface Props {
  title: string;
  icon?: Component;
  active?: boolean;
  expanded?: boolean;
  disabled?: boolean;
  collapsible?: boolean;
  selectable?: boolean;
}

interface Emits {
  (e: 'toggle'): void;
  (e: 'select'): void;
}

const props = withDefaults(defineProps<Props>(), {
  active: false,
  expanded: false,
  disabled: false,
  collapsible: true,
  selectable: true,
});

const emit = defineEmits<Emits>();

const expandIcon = computed(() => (props.expanded ? '▼' : '▶'));

function handleTitleClick() {
  if (props.disabled) return;
  if (props.selectable) emit('select');
  if (props.collapsible) emit('toggle');
}

function handleExpandClick() {
  if (!props.disabled) {
    emit('toggle');
  }
}

function handleRadioChange(event: Event) {
  if (props.disabled) return;
  emit('select');
  restoreProviderRadioState(event.currentTarget as HTMLInputElement, props.active);
}
</script>

<template>
  <div
    class="provider-card"
    :class="{
      active,
      disabled,
      'provider-card--static': !collapsible && !selectable,
    }"
  >
    <div class="card-header" @click="handleTitleClick">
      <input
        v-if="selectable"
        type="radio"
        :checked="active"
        @change="handleRadioChange"
        @click.stop
        :disabled="disabled"
      />
      <component v-if="icon" :is="icon" :size="18" class="provider-icon" />
      <span class="card-title">{{ title }}</span>
      <div class="header-tail">
        <span
          v-if="$slots['header-actions']"
          class="header-actions"
          @click.stop
        >
          <slot name="header-actions" />
        </span>
        <span
          v-if="collapsible"
          class="expand-icon"
          @click.stop="handleExpandClick"
        >{{ expandIcon }}</span>
      </div>
    </div>

    <div v-if="!collapsible || expanded" class="card-content">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.provider-card {
  border: 1px solid var(--color-border);
  border-radius: 12px;
  background: var(--color-bg-field);
  backdrop-filter: blur(8px);
  transition: all 0.2s ease;
}

.provider-card.active {
  border-color: var(--card-active-border);
  background: var(--card-active-bg);
}

.provider-card.disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.card-header {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  user-select: none;
  cursor: pointer;
}

.provider-card:not(.disabled) .card-header:hover {
  background: var(--color-border-weak);
}

.provider-card--static .card-header {
  cursor: default;
}

.provider-card--static:not(.disabled) .card-header:hover {
  background: transparent;
}

.provider-icon {
  color: var(--color-accent);
  flex-shrink: 0;
}

.card-title {
  font-weight: 600;
  font-size: 1.1rem;
  color: var(--color-text-primary);
}

.header-tail {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
  min-width: 0;
}

.header-actions {
  display: flex;
  align-items: center;
}

.expand-icon {
  color: var(--color-text-secondary);
  font-size: 12px;
  cursor: pointer;
}

.card-content {
  padding: 0 16px 12px;
  border-top: 1px solid var(--color-border);
}
</style>
