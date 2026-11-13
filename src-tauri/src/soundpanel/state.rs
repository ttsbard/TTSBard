//! Sound Panel State
//!
//! Управление состоянием звуковой панели: привязки клавиш, флаг перехвата.

use crate::config::{
    AudioSettings, DEFAULT_FLOATING_BG_COLOR, DEFAULT_FLOATING_OPACITY, MAX_FLOATING_OPACITY,
    MIN_FLOATING_OPACITY,
};
use crate::events::AppEvent;
use crate::soundpanel::intercept::InterceptSettings;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};
use uuid::Uuid;

/// Привязка звука к клавише
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundBinding {
    /// Клавиша (A-Z)
    pub key: char,
    /// Описание звука
    pub description: String,
    /// Имя файла в папке soundpanel
    pub filename: String,
    /// Оригинальный путь к файлу (для информации)
    pub original_path: Option<String>,
}

/// Набор звуков (Set) — группа привязок A-Z
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SoundSet {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub bindings: Vec<SoundBinding>,
}

/// Контейнер всех наборов + ID активного набора
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SoundSets {
    #[serde(default)]
    pub active_set_id: String,
    #[serde(default)]
    pub sets: Vec<SoundSet>,
}

impl SoundSets {
    /// Найти активный набор по active_set_id, с fallback на первый
    pub fn find_active(&self) -> Option<&SoundSet> {
        if !self.active_set_id.is_empty() {
            if let Some(set) = self.sets.iter().find(|s| s.id == self.active_set_id) {
                return Some(set);
            }
        }
        self.sets.first()
    }

    /// Найти индекс активного набора, с fallback на 0
    pub fn find_active_index(&self) -> usize {
        self.sets
            .iter()
            .position(|s| s.id == self.active_set_id)
            .unwrap_or_default()
    }

    /// Добавить/заменить привязку в активном наборе (мутация клона)
    pub fn add_active_binding(&mut self, binding: SoundBinding) {
        let idx = self.find_active_index();
        if let Some(active) = self.sets.get_mut(idx) {
            active.bindings.retain(|b| b.key != binding.key);
            active.bindings.push(binding);
        }
    }

    /// Удалить привязку из активного набора (мутация клона)
    pub fn remove_active_binding(&mut self, key: char) {
        let idx = self.find_active_index();
        if let Some(active) = self.sets.get_mut(idx) {
            active.bindings.retain(|b| b.key != key);
        }
    }

    /// Сделать набор с указанным ID активным.
    ///
    /// Возвращает `false`, если набора с таким ID нет: состояние не меняется.
    pub fn set_active(&mut self, id: &str) -> bool {
        if self.sets.iter().any(|s| s.id == id) {
            self.active_set_id = id.to_string();
            true
        } else {
            false
        }
    }

    /// Добавить новый набор и сделать его активным (мутация клона).
    pub fn add_set(&mut self, name: &str) -> SoundSet {
        let set = SoundSet {
            id: gen_set_id(),
            name: name.to_string(),
            bindings: Vec::new(),
        };
        self.active_set_id = set.id.clone();
        self.sets.push(set.clone());
        set
    }

    /// Переименовать набор (мутация клона). Неизвестный ID игнорируется.
    pub fn rename_set(&mut self, id: &str, name: &str) {
        if let Some(set) = self.sets.iter_mut().find(|s| s.id == id) {
            set.name = name.to_string();
        }
    }

    /// Удалить набор (мутация клона). Если удалён активный — переключить на
    /// соседний/предыдущий/пустой. Неизвестный ID игнорируется.
    pub fn remove_set(&mut self, id: &str) {
        let Some(idx) = self.sets.iter().position(|s| s.id == id) else {
            return;
        };

        self.sets.remove(idx);

        if self.active_set_id == id {
            self.active_set_id = if let Some(next) = self.sets.get(idx) {
                next.id.clone()
            } else if idx > 0 {
                self.sets[idx - 1].id.clone()
            } else {
                String::new()
            };
        }
    }

    /// Ссылается ли хотя бы один набор на указанный аудиофайл
    pub fn references_filename(&self, filename: &str) -> bool {
        self.sets
            .iter()
            .any(|s| s.bindings.iter().any(|b| b.filename == filename))
    }
}

/// Максимальное число элементов в очереди воспроизведения SoundPanel.
const SOUND_QUEUE_CAPACITY: usize = 16;

/// Интервал ожидания worker-а между проверками `CancellationToken`.
const SOUND_QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Элемент очереди SoundPanel: путь к аудиофайлу и неизменяемый снимок настроек.
#[derive(Debug, Clone)]
struct QueueItem {
    path: String,
    audio_settings: AudioSettings,
}

/// Bounded FIFO-очередь воспроизведения SoundPanel (sender-сторона).
///
/// Очередь runtime-only: без persistence, UI списка и ручного управления.
/// Переполнение или отсутствие worker-а отклоняют новый элемент с ошибкой.
#[derive(Clone)]
struct SoundQueue {
    tx: SyncSender<QueueItem>,
}

impl SoundQueue {
    fn new() -> (Self, Receiver<QueueItem>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(SOUND_QUEUE_CAPACITY);
        (Self { tx }, rx)
    }

    fn try_enqueue(&self, item: QueueItem) -> Result<(), String> {
        match self.tx.try_send(item) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err("SoundPanel queue is full".to_string()),
            Err(TrySendError::Disconnected(_)) => {
                Err("SoundPanel worker is not running".to_string())
            }
        }
    }
}

/// Worker очереди SoundPanel: FIFO-воспроизведение через `play_audio_file`.
///
/// Синхронно воспроизводит один элемент и только затем берёт следующий.
/// Завершается при отмене `shutdown`, не начиная новый элемент после отмены.
fn run_queue_worker(receiver: Receiver<QueueItem>, shutdown: CancellationToken) {
    run_queue_worker_with(receiver, shutdown, |item| {
        super::audio::play_audio_file(&item.path, &item.audio_settings);
    });
}

/// Testable-ядро worker-а: FIFO-потребление с injectable-функцией воспроизведения.
fn run_queue_worker_with<F>(receiver: Receiver<QueueItem>, shutdown: CancellationToken, mut play: F)
where
    F: FnMut(QueueItem),
{
    loop {
        if shutdown.is_cancelled() {
            info!(
                target = "soundpanel::queue",
                "Queue worker exiting on shutdown"
            );
            return;
        }

        match receiver.recv_timeout(SOUND_QUEUE_POLL_INTERVAL) {
            Ok(item) => {
                if shutdown.is_cancelled() {
                    info!(
                        target = "soundpanel::queue",
                        "Queue worker exiting before next item"
                    );
                    return;
                }
                play(item);
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                info!(
                    target = "soundpanel::queue",
                    "Queue channel disconnected; worker exiting"
                );
                return;
            }
        }
    }
}

/// Состояние звуковой панели
#[derive(Clone)]
pub struct SoundPanelState {
    /// Наборы звуков (Set), каждый со своими привязками
    pub sets: Arc<Mutex<SoundSets>>,

    /// Отправитель событий для MPSC канала
    pub event_sender: Arc<Mutex<Option<Sender<AppEvent>>>>,

    /// Путь к папке %APPDATA%
    pub appdata_path: Arc<Mutex<String>>,

    /// Прозрачность floating окна (10-100)
    pub floating_opacity: Arc<Mutex<u8>>,

    /// Цвет фона floating окна (hex #RRGGBB)
    pub floating_bg_color: Arc<Mutex<String>>,

    /// Пропускает ли floating окно клики
    pub floating_clickthrough: Arc<Mutex<bool>>,

    /// Intercept-настройки (NumPad/F-keys → actions, persisted)
    intercept: Arc<Mutex<InterceptSettings>>,

    /// Writer gate для сериализации мутаций перехвата между clones
    intercept_writer_gate: Arc<Mutex<()>>,

    /// Writer gate для сериализации persist-first мутаций наборов между clones
    sets_writer_gate: Arc<Mutex<()>>,

    /// When enabled, hide_on_blur is bypassed so the panel can persist and be
    /// dragged by its title bar.
    pub stay_visible: Arc<Mutex<bool>>,

    /// Transient runtime flag: while config mode is active, blur-based hiding
    /// is suppressed (e.g. opening the native file picker must not hide the
    /// panel). Never persisted.
    pub config_mode: Arc<Mutex<bool>>,

    /// Transient focus flag used by the keyboard hook to let the active
    /// SoundPanel handle F1-F12 itself.
    window_focused: Arc<AtomicBool>,

    /// Transient recording flag: while key recording is active the low-level
    /// hook passes every key through instead of intercepting bound actions.
    /// Never persisted.
    recording: Arc<AtomicBool>,

    /// Очередь воспроизведения SoundPanel (sender-сторона).
    queue: SoundQueue,

    /// Приёмник очереди, изымается ровно один раз при запуске worker-а.
    queue_receiver: Arc<Mutex<Option<Receiver<QueueItem>>>>,
}

fn gen_set_id() -> String {
    Uuid::new_v4().to_string()
}

impl SoundPanelState {
    /// Создать новое состояние звуковой панели
    pub fn new(appdata_path: String) -> Self {
        let intercept = crate::soundpanel::intercept::load(&appdata_path);
        let (queue, queue_receiver) = SoundQueue::new();
        Self {
            sets: Arc::new(Mutex::new(SoundSets::default())),
            event_sender: Arc::new(Mutex::new(None)),
            appdata_path: Arc::new(Mutex::new(appdata_path)),
            floating_opacity: Arc::new(Mutex::new(DEFAULT_FLOATING_OPACITY)),
            floating_bg_color: Arc::new(Mutex::new(DEFAULT_FLOATING_BG_COLOR.to_string())),
            floating_clickthrough: Arc::new(Mutex::new(false)),
            intercept: Arc::new(Mutex::new(intercept)),
            intercept_writer_gate: Arc::new(Mutex::new(())),
            sets_writer_gate: Arc::new(Mutex::new(())),
            stay_visible: Arc::new(Mutex::new(false)),
            config_mode: Arc::new(Mutex::new(false)),
            window_focused: Arc::new(AtomicBool::new(false)),
            recording: Arc::new(AtomicBool::new(false)),
            queue,
            queue_receiver: Arc::new(Mutex::new(Some(queue_receiver))),
        }
    }

    /// Получить привязку по клавише из активного набора
    pub fn get_binding(&self, key: char) -> Option<SoundBinding> {
        self.sets.lock().ok().and_then(|sets| {
            sets.find_active()
                .and_then(|active| active.bindings.iter().find(|b| b.key == key).cloned())
        })
    }

    /// Получить все привязки активного набора (отсортированные)
    pub fn get_all_bindings(&self) -> Vec<SoundBinding> {
        self.sets
            .lock()
            .ok()
            .and_then(|sets| {
                sets.find_active().map(|active| {
                    let mut bindings: Vec<_> = active.bindings.clone();
                    bindings.sort_by_key(|a| a.key);
                    bindings
                })
            })
            .unwrap_or_default()
    }

    /// Получить все наборы (клон)
    pub fn get_sets(&self) -> SoundSets {
        self.sets.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Получить активный набор (клон, с fallback на пустой)
    pub fn get_active_set(&self) -> SoundSet {
        self.sets
            .lock()
            .ok()
            .and_then(|sets| sets.find_active().cloned())
            .unwrap_or_default()
    }

    /// Сменить активный набор по ID (persist-first).
    ///
    /// Неизвестный ID не меняет состояние.
    pub fn set_active_set(&self, id: &str) -> Result<(), String> {
        self.commit_sets_mutation(|sets| {
            sets.set_active(id);
        })?;
        Ok(())
    }

    /// Создать новый набор и сделать его активным (persist-first).
    pub fn add_set(&self, name: &str) -> Result<SoundSet, String> {
        let mut created = None;
        self.commit_sets_mutation(|sets| {
            created = Some(sets.add_set(name));
        })?;
        created.ok_or_else(|| "Set was not created".to_string())
    }

    /// Переименовать набор (persist-first).
    pub fn rename_set(&self, id: &str, name: &str) -> Result<(), String> {
        self.commit_sets_mutation(|sets| sets.rename_set(id, name))?;
        Ok(())
    }

    /// Удалить набор (persist-first). Если удалён активный — переключить на
    /// соседний/первый.
    pub fn remove_set(&self, id: &str) -> Result<(), String> {
        self.commit_sets_mutation(|sets| sets.remove_set(id))?;
        Ok(())
    }

    /// Захватить writer gate наборов.
    ///
    /// Позволяет командам с привязками (staged-аудио + persist + publish)
    /// выполнить всю мутацию набора под тем же gate, что и set-команды.
    pub fn lock_sets_writer(&self) -> Result<MutexGuard<'_, ()>, String> {
        self.sets_writer_gate
            .lock()
            .map_err(|e| format!("Writer lock error: {}", e))
    }

    /// Persist-first транзакция наборов.
    ///
    /// 1. Под writer gate снимает копию runtime-наборов.
    /// 2. Применяет мутацию к копии, не трогая runtime.
    /// 3. Сохраняет кандидатный снимок на диск.
    /// 4. Только после успешного сохранения публикует снимок в runtime.
    ///
    /// Readers (`get_sets`, `get_all_bindings`) не берут writer gate и никогда
    /// не блокируются на disk I/O; при ошибке записи runtime остаётся прежним.
    fn commit_sets_mutation<M>(&self, mutate: M) -> Result<SoundSets, String>
    where
        M: FnOnce(&mut SoundSets),
    {
        let _writer_guard = self.lock_sets_writer()?;

        let mut next_sets = self.get_sets();
        mutate(&mut next_sets);

        crate::soundpanel::storage::save_sets(self, &next_sets)?;

        self.publish_sets(next_sets.clone());

        Ok(next_sets)
    }

    /// Построить кандидатный снимок наборов с добавленной/заменённой привязкой
    /// в активном наборе. Не изменяет runtime-состояние.
    pub fn candidate_add_binding(&self, binding: &SoundBinding) -> SoundSets {
        let mut sets = self.get_sets();
        sets.add_active_binding(binding.clone());
        sets
    }

    /// Построить кандидатный снимок наборов без привязки в активном наборе.
    /// Не изменяет runtime-состояние.
    pub fn candidate_remove_binding(&self, key: char) -> SoundSets {
        let mut sets = self.get_sets();
        sets.remove_active_binding(key);
        sets
    }

    /// Опубликовать кандидатный снимок в runtime-состояние. Вызывается только
    /// после успешного долговечного сохранения.
    pub fn publish_sets(&self, new_sets: SoundSets) {
        if let Ok(mut sets) = self.sets.lock() {
            *sets = new_sets;
        } else {
            error!(target = "soundpanel::state", "Failed to lock sets");
        }
    }

    /// Воспроизвести звук по привязке: строит путь и ставит элемент в очередь.
    ///
    /// Не создаёт поток на каждый звук и не блокирует вызов. Возвращает ошибку
    /// при переполнении очереди или недоступном worker-е.
    pub fn play_sound(
        &self,
        binding: &SoundBinding,
        audio_settings: AudioSettings,
    ) -> Result<(), String> {
        let appdata_path = self
            .appdata_path
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .clone();
        let sound_path = format!("{}\\soundpanel\\{}", appdata_path, binding.filename);

        info!(target = "soundpanel", key = %binding.key, path = ?sound_path, "Enqueueing sound");

        self.enqueue_sound(sound_path, audio_settings)
    }

    /// Поставить аудиофайл в очередь воспроизведения SoundPanel.
    pub fn enqueue_sound(&self, path: String, audio_settings: AudioSettings) -> Result<(), String> {
        self.queue.try_enqueue(QueueItem {
            path,
            audio_settings,
        })
    }

    /// Запустить единственного worker-а очереди воспроизведения SoundPanel.
    ///
    /// Worker забирает элементы FIFO и синхронно воспроизводит каждый через
    /// `soundpanel::audio::play_audio_file`, не создавая поток на каждый звук.
    /// Завершается при отмене переданного `CancellationToken`. Вызывается ровно
    /// один раз из `setup::init_app`.
    pub fn start_queue_worker(&self, shutdown: CancellationToken) -> Result<(), String> {
        let receiver = self
            .queue_receiver
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .take()
            .ok_or_else(|| "SoundPanel queue worker already started".to_string())?;

        std::thread::spawn(move || {
            run_queue_worker(receiver, shutdown);
        });

        info!(
            target = "soundpanel::queue",
            "SoundPanel queue worker started"
        );
        Ok(())
    }

    /// Установить отправитель событий
    pub fn set_event_sender(&self, sender: Sender<AppEvent>) {
        if let Ok(mut es) = self.event_sender.lock() {
            *es = Some(sender);
        } else {
            error!(target = "soundpanel::state", "Failed to lock event_sender");
        }
    }

    /// Отправить событие
    pub fn emit_event(&self, event: AppEvent) {
        debug!(target = "soundpanel::state", event = ?std::mem::discriminant(&event), "emit_event called");
        if let Ok(es) = self.event_sender.lock() {
            if let Some(ref sender) = *es {
                debug!(
                    target = "soundpanel::state",
                    "Sending event through channel"
                );
                match sender.send(event) {
                    Ok(_) => debug!(target = "soundpanel::state", "Event sent successfully"),
                    Err(error) => {
                        error!(target = "soundpanel::state", error = %error, "Failed to send event")
                    }
                }
            } else {
                error!(target = "soundpanel::state", "event_sender is None");
            }
        } else {
            error!(target = "soundpanel::state", "Failed to lock event_sender");
        }
    }

    /// Получить прозрачность floating окна
    pub fn get_floating_opacity(&self) -> u8 {
        self.floating_opacity
            .lock()
            .map(|v| *v)
            .unwrap_or(DEFAULT_FLOATING_OPACITY)
    }

    /// Установить прозрачность floating окна
    pub fn set_floating_opacity(&self, value: u8) {
        debug!(
            target = "soundpanel::state",
            value, "set_floating_opacity called"
        );
        if let Ok(mut val) = self.floating_opacity.lock() {
            *val = value.clamp(MIN_FLOATING_OPACITY, MAX_FLOATING_OPACITY);
            debug!(
                target = "soundpanel::state",
                opacity = *val,
                "Opacity updated"
            );
        } else {
            error!(
                target = "soundpanel::state",
                "Failed to lock floating_opacity"
            );
            return;
        }
        debug!(
            target = "soundpanel::state",
            "Emitting SoundPanelAppearanceChanged event"
        );
        self.emit_event(AppEvent::SoundPanelAppearanceChanged);
        debug!(target = "soundpanel::state", "Event emitted");
    }

    /// Получить цвет фона floating окна
    pub fn get_floating_bg_color(&self) -> String {
        self.floating_bg_color
            .lock()
            .map(|v| v.clone())
            .unwrap_or_else(|_| DEFAULT_FLOATING_BG_COLOR.to_string())
    }

    /// Установить цвет фона floating окна
    pub fn set_floating_bg_color(&self, color: String) {
        debug!(
            target = "soundpanel::state",
            color, "set_floating_bg_color called"
        );
        let color_clone = color.clone();
        if let Ok(mut val) = self.floating_bg_color.lock() {
            *val = color_clone.clone();
            debug!(target = "soundpanel::state", color = ?color_clone, "Color updated");
        } else {
            error!(
                target = "soundpanel::state",
                "Failed to lock floating_bg_color"
            );
            return;
        }
        debug!(
            target = "soundpanel::state",
            "Emitting SoundPanelAppearanceChanged event"
        );
        self.emit_event(AppEvent::SoundPanelAppearanceChanged);
        debug!(target = "soundpanel::state", "Event emitted");
    }

    /// Проверить, включен ли clickthrough для floating окна
    pub fn is_floating_clickthrough_enabled(&self) -> bool {
        self.floating_clickthrough
            .lock()
            .map(|v| *v)
            .unwrap_or(false)
    }

    /// Установить clickthrough для floating окна
    pub fn set_floating_clickthrough(&self, enabled: bool) {
        debug!(
            target = "soundpanel::state",
            enabled, "set_floating_clickthrough called"
        );
        if let Ok(mut val) = self.floating_clickthrough.lock() {
            *val = enabled;
            debug!(
                target = "soundpanel::state",
                enabled, "Clickthrough updated"
            );
        } else {
            error!(
                target = "soundpanel::state",
                "Failed to lock floating_clickthrough"
            );
            return;
        }
        self.emit_event(AppEvent::SoundPanelAppearanceChanged);
    }

    /// Проверить, включен ли stay_visible
    pub fn get_stay_visible(&self) -> bool {
        self.stay_visible.lock().map(|v| *v).unwrap_or(false)
    }

    /// Установить stay_visible
    pub fn set_stay_visible(&self, enabled: bool) {
        if let Ok(mut val) = self.stay_visible.lock() {
            *val = enabled;
        } else {
            error!(target = "soundpanel::state", "Failed to lock stay_visible");
        }
    }

    /// Проверить, активен ли config-режим
    pub fn get_config_mode(&self) -> bool {
        self.config_mode.lock().map(|v| *v).unwrap_or(false)
    }

    /// Установить config-режим (транзитный флаг, не сохраняется на диск)
    pub fn set_config_mode(&self, enabled: bool) {
        if let Ok(mut val) = self.config_mode.lock() {
            *val = enabled;
        } else {
            error!(target = "soundpanel::state", "Failed to lock config_mode");
        }
    }

    pub fn is_window_focused(&self) -> bool {
        self.window_focused.load(Ordering::Acquire)
    }

    pub fn set_window_focused(&self, focused: bool) {
        self.window_focused.store(focused, Ordering::Release);
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Acquire)
    }

    pub fn set_recording(&self, recording: bool) {
        self.recording.store(recording, Ordering::Release);
    }

    /// Получить настройки перехвата (clone)
    pub fn get_intercept(&self) -> InterceptSettings {
        self.intercept.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// Общий transaction helper для всех мутаций перехвата.
    ///
    /// 1. Под writer gate кратко снимает копию текущего runtime.
    /// 2. Отпускает runtime lock на время сохранения на диск (`save`).
    /// 3. После успешного сохранения кратко захватывает runtime lock и публикует изменения.
    /// 4. Выполняет `post_commit` под writer gate (обеспечивая строгий порядок публикации и событий).
    ///
    /// Readers (`get_intercept`) не берут writer gate и никогда не блокируются на disk I/O.
    fn mutate_intercept_with<M, S, P>(
        &self,
        mutate: M,
        save: S,
        post_commit: P,
    ) -> Result<(), String>
    where
        M: FnOnce(&mut InterceptSettings),
        S: FnOnce(&str, &InterceptSettings) -> Result<(), String>,
        P: FnOnce(&InterceptSettings),
    {
        let _writer_guard = self
            .intercept_writer_gate
            .lock()
            .map_err(|e| format!("Writer lock error: {}", e))?;

        let mut next_settings = {
            let runtime_guard = self
                .intercept
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            runtime_guard.clone()
        };

        mutate(&mut next_settings);

        let appdata_path = self
            .appdata_path
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .clone();

        save(&appdata_path, &next_settings)?;

        {
            let mut runtime_guard = self
                .intercept
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            *runtime_guard = next_settings.clone();
        }

        post_commit(&next_settings);

        Ok(())
    }

    /// Включить/выключить перехват (persist + emit)
    pub fn set_intercept_enabled(&self, enabled: bool) -> Result<(), String> {
        self.set_intercept_enabled_with(enabled, crate::soundpanel::intercept::save)
    }

    pub(crate) fn set_intercept_enabled_with<S>(&self, enabled: bool, save: S) -> Result<(), String>
    where
        S: FnOnce(&str, &InterceptSettings) -> Result<(), String>,
    {
        self.mutate_intercept_with(
            |settings| {
                settings.enabled = enabled;
            },
            save,
            |settings| {
                self.emit_event(AppEvent::InterceptionChanged(settings.enabled));
            },
        )
    }

    /// Включить/выключить unrestricted-перехват (persist + publish).
    ///
    /// Меняет только `allow_any_key`, сохраняя `enabled` и `bindings`.
    /// Persist выполняется до публикации в runtime, поэтому при ошибке записи
    /// runtime-состояние остаётся неизменным. Фронтенд после вызова сеттера
    /// дожидается результата и перечитывает настройки через `get_intercept_settings`.
    pub fn set_intercept_allow_any_key(&self, allow_any_key: bool) -> Result<(), String> {
        self.set_intercept_allow_any_key_with(allow_any_key, crate::soundpanel::intercept::save)
    }

    pub(crate) fn set_intercept_allow_any_key_with<S>(
        &self,
        allow_any_key: bool,
        save: S,
    ) -> Result<(), String>
    where
        S: FnOnce(&str, &InterceptSettings) -> Result<(), String>,
    {
        self.mutate_intercept_with(
            |settings| {
                settings.allow_any_key = allow_any_key;
            },
            save,
            |_| {
                info!(allow_any_key, "Intercept allow_any_key set");
            },
        )
    }

    /// Установить биндинг перехвата (persist)
    pub fn set_intercept_binding(&self, key: String, action: String) -> Result<(), String> {
        self.set_intercept_binding_with(key, action, crate::soundpanel::intercept::save)
    }

    pub(crate) fn set_intercept_binding_with<S>(
        &self,
        key: String,
        action: String,
        save: S,
    ) -> Result<(), String>
    where
        S: FnOnce(&str, &InterceptSettings) -> Result<(), String>,
    {
        self.mutate_intercept_with(
            |settings| {
                settings.bindings.retain(|b| b.key != key);
                settings
                    .bindings
                    .push(crate::soundpanel::intercept::InterceptBinding {
                        key: key.clone(),
                        action: action.clone(),
                    });
            },
            save,
            |_| {
                info!(key = key, action = action, "Intercept binding set");
            },
        )
    }

    /// Очистить биндинг перехвата (persist)
    pub fn clear_intercept_binding(&self, key: String) -> Result<(), String> {
        self.clear_intercept_binding_with(key, crate::soundpanel::intercept::save)
    }

    pub(crate) fn clear_intercept_binding_with<S>(&self, key: String, save: S) -> Result<(), String>
    where
        S: FnOnce(&str, &InterceptSettings) -> Result<(), String>,
    {
        self.mutate_intercept_with(
            |settings| {
                settings.bindings.retain(|b| b.key != key);
            },
            save,
            |_| {
                info!(key = key, "Intercept binding cleared");
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn bad_path(label: &str) -> String {
        std::env::temp_dir()
            .join(format!(
                "ttsbard_state_test_{}_{}",
                std::process::id(),
                label,
            ))
            .join("nonexistent")
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn test_find_active_fallback() {
        let sets = SoundSets {
            active_set_id: "bogus".into(),
            sets: vec![SoundSet {
                id: "set1".into(),
                name: "First".into(),
                bindings: vec![],
            }],
        };
        let active = sets.find_active();
        assert!(active.is_some());
        assert_eq!(active.unwrap().id, "set1");
    }

    #[test]
    fn test_find_active_empty() {
        let sets = SoundSets::default();
        assert!(sets.find_active().is_none());
    }

    #[test]
    fn test_find_active_index_not_first() {
        let sets = SoundSets {
            active_set_id: "set2".into(),
            sets: vec![
                SoundSet {
                    id: "set1".into(),
                    name: "First".into(),
                    bindings: vec![],
                },
                SoundSet {
                    id: "set2".into(),
                    name: "Second".into(),
                    bindings: vec![],
                },
            ],
        };
        assert_eq!(sets.find_active_index(), 1);
    }

    #[test]
    fn test_find_active_index_invalid_falls_back_to_zero() {
        let sets = SoundSets {
            active_set_id: "bogus".into(),
            sets: vec![SoundSet {
                id: "set1".into(),
                name: "First".into(),
                bindings: vec![],
            }],
        };
        assert_eq!(sets.find_active_index(), 0);
    }

    #[test]
    fn references_filename_protects_file_used_in_other_set() {
        let mut sets = SoundSets {
            active_set_id: "set1".into(),
            sets: vec![
                SoundSet {
                    id: "set1".into(),
                    name: "First".into(),
                    bindings: vec![SoundBinding {
                        key: 'A',
                        description: "removed".into(),
                        filename: "shared.mp3".into(),
                        original_path: None,
                    }],
                },
                SoundSet {
                    id: "set2".into(),
                    name: "Second".into(),
                    bindings: vec![SoundBinding {
                        key: 'B',
                        description: "other".into(),
                        filename: "shared.mp3".into(),
                        original_path: None,
                    }],
                },
            ],
        };

        sets.remove_active_binding('A');

        assert!(
            sets.references_filename("shared.mp3"),
            "filename still referenced by another set must be protected"
        );
        assert!(!sets.references_filename("unused.mp3"));
    }

    #[test]
    fn test_find_active_index_empty_active_set_id_returns_zero() {
        let sets = SoundSets {
            active_set_id: String::new(),
            sets: vec![
                SoundSet {
                    id: "set1".into(),
                    name: "First".into(),
                    bindings: vec![],
                },
                SoundSet {
                    id: "set2".into(),
                    name: "Second".into(),
                    bindings: vec![],
                },
            ],
        };
        assert_eq!(sets.find_active_index(), 0);
    }

    #[test]
    fn test_migration_vec_to_sets() {
        let old_json = r#"[
            {"key":"A","description":"test a","filename":"a.mp3","original_path":null},
            {"key":"B","description":"test b","filename":"b.mp3","original_path":"D:\\b.mp3"}
        ]"#;

        let old_bindings: Vec<SoundBinding> = serde_json::from_str(old_json).unwrap();
        assert_eq!(old_bindings.len(), 2);

        let id = gen_set_id();
        let sets = SoundSets {
            active_set_id: id.clone(),
            sets: vec![SoundSet {
                id,
                name: "Основной".into(),
                bindings: old_bindings,
            }],
        };

        let active = sets.find_active().unwrap();
        assert_eq!(active.name, "Основной");
        assert_eq!(active.bindings.len(), 2);
        assert_eq!(active.bindings[0].key, 'A');
    }

    // ── Sets persistence failure safety ─────────────────────────────────

    fn seeded_sets() -> SoundSets {
        SoundSets {
            active_set_id: "set1".into(),
            sets: vec![
                SoundSet {
                    id: "set1".into(),
                    name: "First".into(),
                    bindings: vec![],
                },
                SoundSet {
                    id: "set2".into(),
                    name: "Second".into(),
                    bindings: vec![],
                },
            ],
        }
    }

    #[test]
    fn set_active_set_persist_failure_leaves_state_unchanged() {
        let state = SoundPanelState::new(bad_path("active_fail"));
        state.publish_sets(seeded_sets());

        let result = state.set_active_set("set2");
        assert!(result.is_err());
        assert_eq!(state.get_sets().active_set_id, "set1");
    }

    #[test]
    fn add_set_persist_failure_leaves_state_unchanged() {
        let state = SoundPanelState::new(bad_path("add_fail"));
        state.publish_sets(seeded_sets());

        let result = state.add_set("Third");
        assert!(result.is_err());
        let sets = state.get_sets();
        assert_eq!(sets.sets.len(), 2);
        assert_eq!(sets.active_set_id, "set1");
    }

    #[test]
    fn rename_set_persist_failure_leaves_state_unchanged() {
        let state = SoundPanelState::new(bad_path("rename_fail"));
        state.publish_sets(seeded_sets());

        let result = state.rename_set("set1", "Renamed");
        assert!(result.is_err());
        assert_eq!(state.get_sets().sets[0].name, "First");
    }

    #[test]
    fn remove_set_persist_failure_leaves_state_unchanged() {
        let state = SoundPanelState::new(bad_path("remove_fail"));
        state.publish_sets(seeded_sets());

        let result = state.remove_set("set1");
        assert!(result.is_err());
        let sets = state.get_sets();
        assert_eq!(sets.sets.len(), 2);
        assert_eq!(sets.active_set_id, "set1");
    }

    #[test]
    fn set_mutations_persist_snapshot_before_publishing() {
        let dir = std::env::temp_dir().join(format!(
            "ttsbard_state_sets_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.to_string_lossy().to_string();

        let state = SoundPanelState::new(path);
        let first = state.add_set("First").unwrap();

        let bindings_file = dir.join("soundpanel_bindings.json");
        let on_disk: SoundSets =
            serde_json::from_str(&std::fs::read_to_string(&bindings_file).unwrap()).unwrap();
        assert_eq!(on_disk.sets.len(), 1);
        assert_eq!(on_disk.sets[0].id, first.id);
        assert_eq!(on_disk.sets[0].name, "First");
        assert_eq!(on_disk.active_set_id, first.id);

        let second = state.add_set("Second").unwrap();
        state.set_active_set(&first.id).unwrap();
        state.rename_set(&second.id, "Renamed").unwrap();
        state.remove_set(&first.id).unwrap();

        let on_disk: SoundSets =
            serde_json::from_str(&std::fs::read_to_string(&bindings_file).unwrap()).unwrap();
        assert_eq!(on_disk.sets.len(), 1);
        assert_eq!(on_disk.sets[0].id, second.id);
        assert_eq!(on_disk.sets[0].name, "Renamed");
        // Removing the active set switches to the only remaining one.
        assert_eq!(on_disk.active_set_id, second.id);
        assert_eq!(state.get_sets().active_set_id, second.id);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sets_writer_gate_is_shared_between_clones() {
        let state = SoundPanelState::new(bad_path("gate_shared"));
        let clone = state.clone();

        let guard = state.lock_sets_writer().unwrap();
        assert!(matches!(
            clone.sets_writer_gate.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ));
        drop(guard);
        assert!(clone.sets_writer_gate.try_lock().is_ok());
    }

    #[test]
    fn remove_active_set_selects_neighbour_or_clears() {
        let mut sets = SoundSets {
            active_set_id: "set2".into(),
            sets: seeded_sets().sets,
        };

        sets.remove_set("set2");
        assert_eq!(sets.sets.len(), 1);
        assert_eq!(sets.active_set_id, "set1");

        sets.remove_set("set1");
        assert!(sets.sets.is_empty());
        assert!(sets.active_set_id.is_empty());
    }

    // ── Intercept persistence failure safety ────────────────────────────

    #[test]
    fn set_intercept_enabled_persist_failure_leaves_state_unchanged() {
        let path = bad_path("enabled_fail");
        let state = SoundPanelState::new(path);
        assert!(!state.get_intercept().enabled);

        let result = state.set_intercept_enabled(true);
        assert!(result.is_err());
        assert!(!state.get_intercept().enabled);
    }

    #[test]
    fn set_intercept_enabled_persist_failure_does_not_emit_event() {
        let path = bad_path("enabled_noevent");
        let state = SoundPanelState::new(path);
        let (tx, rx) = mpsc::channel();
        state.set_event_sender(tx);

        let result = state.set_intercept_enabled(true);
        assert!(result.is_err());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn set_intercept_binding_persist_failure_leaves_state_unchanged() {
        let path = bad_path("binding_fail");
        let state = SoundPanelState::new(path);
        assert!(state.get_intercept().bindings.is_empty());

        let result = state.set_intercept_binding("NUMPAD1".into(), "play_sound".into());
        assert!(result.is_err());
        assert!(state.get_intercept().bindings.is_empty());
    }

    #[test]
    fn clear_intercept_binding_persist_failure_leaves_state_unchanged() {
        let path = bad_path("clear_fail");
        let state = SoundPanelState::new(path);
        // Pre-populate runtime (without persist) so we can test rollback
        {
            let mut val = state.intercept.lock().unwrap();
            val.bindings
                .push(crate::soundpanel::intercept::InterceptBinding {
                    key: "NUMPAD1".into(),
                    action: "play_sound".into(),
                });
        }
        assert_eq!(state.get_intercept().bindings.len(), 1);

        let result = state.clear_intercept_binding("NUMPAD1".into());
        assert!(result.is_err());
        assert_eq!(state.get_intercept().bindings.len(), 1);
    }

    #[test]
    fn set_intercept_allow_any_key_persist_failure_leaves_state_unchanged() {
        let path = bad_path("allow_any_fail");
        let state = SoundPanelState::new(path);
        assert!(!state.get_intercept().allow_any_key);

        let result = state.set_intercept_allow_any_key(true);
        assert!(result.is_err());
        assert!(!state.get_intercept().allow_any_key);
    }

    #[test]
    fn set_intercept_allow_any_key_preserves_enabled_and_bindings() {
        let dir = std::env::temp_dir().join(format!(
            "ttsbard_state_allow_any_ok_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.to_string_lossy().to_string();

        let state = SoundPanelState::new(path.clone());
        state.set_intercept_enabled(true).unwrap();
        state
            .set_intercept_binding("HOME".into(), "playback_stop".into())
            .unwrap();

        state.set_intercept_allow_any_key(true).unwrap();

        let settings = state.get_intercept();
        assert!(settings.allow_any_key);
        assert!(settings.enabled);
        assert_eq!(settings.bindings.len(), 1);
        assert_eq!(settings.bindings[0].key, "HOME");
        assert_eq!(settings.bindings[0].action, "playback_stop");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reader_observes_old_runtime_while_writer_is_persisting() {
        let dir = std::env::temp_dir().join(format!(
            "ttsbard_test_reader_nonblocking_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.to_string_lossy().to_string();

        let state = SoundPanelState::new(path);
        assert!(!state.get_intercept().enabled);

        let (save_started_tx, save_started_rx) = mpsc::channel::<()>();
        let (save_unblock_tx, save_unblock_rx) = mpsc::channel::<()>();

        let state_clone = state.clone();
        let writer_handle = std::thread::spawn(move || {
            state_clone.set_intercept_enabled_with(true, |_path, _settings| {
                let _ = save_started_tx.send(());
                save_unblock_rx
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|e| format!("Save timeout: {}", e))?;
                Ok(())
            })
        });

        // Writer enters save callback (holding writer gate, but runtime lock dropped)
        save_started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("writer must enter save callback");

        // Reader accesses get_intercept() while save is stalled.
        // It must NOT block, and must observe the uncommitted previous state.
        let observed = state.get_intercept();
        assert!(
            !observed.enabled,
            "reader must observe previous settings during disk I/O"
        );

        // Unblock writer and verify completion
        let _ = save_unblock_tx.send(());
        let writer_result = writer_handle.join().expect("writer thread should join");
        assert!(writer_result.is_ok(), "writer should complete successfully");

        assert!(
            state.get_intercept().enabled,
            "reader must observe updated settings after commit"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_writers_across_clones_preserve_both_mutations_without_lost_updates() {
        let dir = std::env::temp_dir().join(format!(
            "ttsbard_test_concurrent_writers_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.to_string_lossy().to_string();

        let state = SoundPanelState::new(path);

        let (w1_started_tx, w1_started_rx) = mpsc::channel::<()>();
        let (w1_unblock_tx, w1_unblock_rx) = mpsc::channel::<()>();

        let state_w1 = state.clone();
        let writer1 = std::thread::spawn(move || {
            state_w1.set_intercept_binding_with(
                "NUMPAD1".into(),
                "action1".into(),
                |_path, settings| {
                    let _ = w1_started_tx.send(());
                    w1_unblock_rx
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|e| format!("Writer 1 timeout: {}", e))?;
                    assert!(settings.bindings.iter().any(|b| b.key == "NUMPAD1"));
                    Ok(())
                },
            )
        });

        // Wait until writer 1 is stalled inside save callback
        w1_started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("writer 1 must enter save");

        // Writer 2 starts mutation on its clone while writer 1 holds the writer gate
        let state_w2 = state.clone();
        let (w2_gate_tx, w2_gate_rx) = mpsc::channel();
        let writer2 = std::thread::spawn(move || {
            // Observe the actual shared transaction gate while writer 1 is in save.
            // This fails if the transaction stops holding its gate during persistence.
            let gate_is_held = matches!(
                state_w2.intercept_writer_gate.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            );
            let _ = w2_gate_tx.send(gate_is_held);
            state_w2.set_intercept_binding_with(
                "NUMPAD2".into(),
                "action2".into(),
                |_path, settings| {
                    assert!(
                        settings.bindings.iter().any(|b| b.key == "NUMPAD1"),
                        "writer 2 must see committed changes from writer 1"
                    );
                    assert!(
                        settings.bindings.iter().any(|b| b.key == "NUMPAD2"),
                        "writer 2 must include its own changes"
                    );
                    Ok(())
                },
            )
        });

        // The handshake proves contention without relying on thread scheduling delays.
        let gate_observation = w2_gate_rx.recv_timeout(Duration::from_secs(2));

        // Always release and join before assertions, including timeout/failure paths.
        let _ = w1_unblock_tx.send(());
        let writer1_result = writer1.join();
        let writer2_result = writer2.join();
        let _ = std::fs::remove_dir_all(&dir);

        assert!(gate_observation.expect("writer 2 must observe the gate"));
        writer1_result.expect("writer 1 join").expect("writer 1 ok");
        writer2_result.expect("writer 2 join").expect("writer 2 ok");

        // Final state must contain both bindings
        let final_settings = state.get_intercept();
        assert_eq!(final_settings.bindings.len(), 2);
        assert!(final_settings
            .bindings
            .iter()
            .any(|b| b.key == "NUMPAD1" && b.action == "action1"));
        assert!(final_settings
            .bindings
            .iter()
            .any(|b| b.key == "NUMPAD2" && b.action == "action2"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_error_leaves_runtime_unchanged_no_event_and_retry_succeeds() {
        let path = bad_path("save_err_retry");
        let state = SoundPanelState::new(path);
        let (event_tx, event_rx) = mpsc::channel();
        state.set_event_sender(event_tx);

        assert!(!state.get_intercept().enabled);

        // Attempt 1: save fails
        let err_result = state.set_intercept_enabled_with(true, |_path, _settings| {
            Err("simulated save error".to_string())
        });
        assert!(err_result.is_err());
        assert!(
            !state.get_intercept().enabled,
            "runtime must remain false after save failure"
        );
        assert!(
            event_rx.try_recv().is_err(),
            "no event emitted on save failure"
        );

        // Attempt 2: retry succeeds
        let ok_result = state.set_intercept_enabled_with(true, |_path, _settings| Ok(()));
        assert!(ok_result.is_ok());
        assert!(
            state.get_intercept().enabled,
            "runtime must be true after retry succeeds"
        );
        match event_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(AppEvent::InterceptionChanged(true)) => {}
            other => panic!("expected InterceptionChanged(true), got {:?}", other),
        }
    }

    // ── SoundPanel playback queue ────────────────────────────────────────

    fn queue_item(path: &str) -> QueueItem {
        QueueItem {
            path: path.to_string(),
            audio_settings: AudioSettings::default(),
        }
    }

    #[test]
    fn queue_worker_processes_items_fifo() {
        let (queue, rx) = SoundQueue::new();
        let token = CancellationToken::new();
        let played = Arc::new(Mutex::new(Vec::new()));
        let played_clone = Arc::clone(&played);

        let handle = std::thread::spawn(move || {
            run_queue_worker_with(rx, token, |item| {
                played_clone.lock().unwrap().push(item.path);
            });
        });

        queue.try_enqueue(queue_item("a")).unwrap();
        queue.try_enqueue(queue_item("b")).unwrap();
        queue.try_enqueue(queue_item("c")).unwrap();
        drop(queue);

        handle.join().unwrap();

        assert_eq!(*played.lock().unwrap(), vec!["a", "b", "c"]);
    }

    #[test]
    fn bounded_queue_rejects_when_full() {
        let (queue, rx) = SoundQueue::new();

        for i in 0..SOUND_QUEUE_CAPACITY {
            queue.try_enqueue(queue_item(&format!("{i}"))).unwrap();
        }

        assert!(queue.try_enqueue(queue_item("overflow")).is_err());

        drop(rx);
        assert!(queue.try_enqueue(queue_item("disconnected")).is_err());
    }

    #[test]
    fn queue_worker_stops_at_shutdown_before_next_item() {
        let (queue, rx) = SoundQueue::new();
        let token = CancellationToken::new();
        let worker_token = token.clone();
        let played = Arc::new(Mutex::new(Vec::new()));
        let played_clone = Arc::clone(&played);

        let (started_tx, started_rx) = mpsc::channel::<()>();

        let handle = std::thread::spawn(move || {
            run_queue_worker_with(rx, worker_token, move |item| {
                played_clone.lock().unwrap().push(item.path);
                let _ = started_tx.send(());
                std::thread::sleep(Duration::from_millis(50));
            });
        });

        queue.try_enqueue(queue_item("a")).unwrap();
        queue.try_enqueue(queue_item("b")).unwrap();

        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("first item must start playing");

        token.cancel();

        handle.join().unwrap();

        assert_eq!(*played.lock().unwrap(), vec!["a"]);
    }
}
