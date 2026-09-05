<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import Icon from "./lib/Icon.svelte";
  import { connectTwitch, disconnectTwitch, getSnapshot, onSnapshot, refreshVips, removeVips, setStreakThreshold } from "./lib/api";
  import { emptySnapshot, previewSnapshot } from "./lib/mock";
  import { MAX_REMOVALS_PER_OPERATION, canAddSelection, channelConfirmationMatches, selectSafeBatch } from "./lib/removalSafety";
  import type { AppSnapshot, ReportUser } from "./lib/types";

  type View = "dashboard" | "vips" | "history" | "settings";
  type InterfaceSize = "compact" | "standard" | "large";
  type FontSize = "normal" | "large" | "xlarge";

  const interfaceZoom: Record<InterfaceSize, number> = { compact: 0.9, standard: 1.1, large: 1.3 };

  let snapshot: AppSnapshot = emptySnapshot;
  let view: View = "dashboard";
  let loading = true;
  let connecting = false;
  let clientId = "";
  let accessToken = "";
  let showToken = false;
  let connectError = "";
  let externalError = "";
  let guideStep = 1;
  let selected = new Set<string>();
  let removing = false;
  let confirmOpen = false;
  let removalConfirmation = "";
  let removalError = "";
  let settingsThreshold = 150;
  let settingsSaved = false;
  let previewMode = false;
  let interfaceSize: InterfaceSize = "standard";
  let fontSize: FontSize = "large";
  let refreshingVips = false;
  let refreshError = "";

  $: activeVips = snapshot.vips.filter((vip) => vip.wroteThisStream).length;
  $: silentVips = snapshot.stream ? snapshot.vips.filter((vip) => !vip.wroteThisStream).length : 0;
  $: selectedReport = snapshot.report.filter((item) => selected.has(item.userId));
  $: confirmationMatches = channelConfirmationMatches(removalConfirmation, snapshot.channelLogin);

  onMount(async () => {
    previewMode = !Object.prototype.hasOwnProperty.call(window, "__TAURI_INTERNALS__") || new URLSearchParams(location.search).has("preview");
    const savedInterface = localStorage.getItem("vipperfox-interface-size");
    const savedFont = localStorage.getItem("vipperfox-font-size");
    if (savedInterface === "compact" || savedInterface === "standard" || savedInterface === "large") interfaceSize = savedInterface;
    if (savedFont === "normal" || savedFont === "large" || savedFont === "xlarge") fontSize = savedFont;
    await applyAppearance();
    if (previewMode) {
      snapshot = previewSnapshot;
      settingsThreshold = snapshot.streakThreshold;
      loading = false;
      return;
    }
    try {
      snapshot = await getSnapshot();
      settingsThreshold = snapshot.streakThreshold;
      await onSnapshot((next) => {
        snapshot = next;
        if (next.stream || !next.reportComplete) {
          selected = new Set();
          confirmOpen = false;
          removalConfirmation = "";
        } else {
          selected = new Set([...selected].filter((id) => next.report.some((item) => item.userId === id)));
        }
      });
    } catch (error) {
      connectError = String(error);
    } finally {
      loading = false;
    }
  });

  async function applyAppearance() {
    document.documentElement.dataset.fontSize = fontSize;
    if (!previewMode) {
      try {
        await getCurrentWebview().setZoom(interfaceZoom[interfaceSize]);
      } catch (error) {
        console.warn("Не удалось применить масштаб интерфейса", error);
      }
    }
  }

  async function chooseInterfaceSize(value: InterfaceSize) {
    interfaceSize = value;
    localStorage.setItem("vipperfox-interface-size", value);
    await applyAppearance();
  }

  async function chooseFontSize(value: FontSize) {
    fontSize = value;
    localStorage.setItem("vipperfox-font-size", value);
    await applyAppearance();
  }

  async function reloadVips() {
    refreshingVips = true;
    refreshError = "";
    try {
      snapshot = previewMode ? snapshot : await refreshVips();
    } catch (error) {
      refreshError = String(error).replace(/^Error:\s*/, "");
    } finally {
      refreshingVips = false;
    }
  }

  async function openExternal(url: string) {
    externalError = "";
    try {
      if (previewMode) window.open(url, "_blank", "noopener,noreferrer");
      else await openUrl(url);
    } catch (error) {
      externalError = `Не удалось открыть браузер: ${String(error).replace(/^Error:\s*/, "")}`;
    }
  }

  async function connect() {
    if (!clientId.trim() || !accessToken.trim()) return;
    connecting = true;
    connectError = "";
    try {
      snapshot = await connectTwitch({ clientId: clientId.trim(), accessToken: accessToken.trim() });
      accessToken = "";
      view = "dashboard";
    } catch (error) {
      connectError = String(error).replace(/^Error:\s*/, "");
    } finally {
      connecting = false;
    }
  }

  function toggleSelected(userId: string) {
    if (!snapshot.reportComplete || removing) return;
    const next = new Set(selected);
    if (!canAddSelection(next, userId)) {
      removalError = `За одну безопасную операцию можно выбрать максимум ${MAX_REMOVALS_PER_OPERATION} пользователей.`;
      return;
    }
    next.has(userId) ? next.delete(userId) : next.add(userId);
    selected = next;
    removalError = "";
  }

  function selectAllReport() {
    if (!snapshot.reportComplete || removing) return;
    const batch = snapshot.report.slice(0, MAX_REMOVALS_PER_OPERATION);
    selected = selected.size === batch.length
      ? new Set()
      : selectSafeBatch(snapshot.report);
    removalError = snapshot.report.length > MAX_REMOVALS_PER_OPERATION ? `Выбраны первые ${MAX_REMOVALS_PER_OPERATION} пользователей. Остальных можно обработать следующим пакетом.` : "";
  }

  async function confirmRemoval() {
    if (!selected.size) return;
    removing = true;
    removalError = "";
    try {
      snapshot = await removeVips([...selected], removalConfirmation);
      selected = new Set();
      confirmOpen = false;
      removalConfirmation = "";
    } catch (error) {
      removalError = String(error).replace(/^Error:\s*/, "");
    } finally {
      removing = false;
    }
  }

  async function saveSettings() {
    snapshot = await setStreakThreshold(Math.max(1, settingsThreshold));
    settingsSaved = true;
    setTimeout(() => settingsSaved = false, 1800);
  }

  function streamDuration() {
    if (!snapshot.stream) return "—";
    const minutes = Math.max(0, Math.floor((Date.now() - new Date(snapshot.stream.startedAt).getTime()) / 60000));
    return `${Math.floor(minutes / 60)} ч ${minutes % 60} мин`;
  }

  function timeLabel(value: string) {
    return new Intl.DateTimeFormat("ru", { hour: "2-digit", minute: "2-digit" }).format(new Date(value));
  }
</script>

<svelte:head><title>VipperFox — Twitch VIP Manager</title></svelte:head>

{#if loading}
  <div class="splash">
    <div class="splash-glow"></div>
    <img src="/fox.png" alt="VipperFox" />
    <div class="splash-word">Vipper<span>Fox</span></div>
    <div class="loading-dots"><i></i><i></i><i></i></div>
  </div>
{:else if !snapshot.configured}
  <main class="onboarding">
    <div class="ambient ambient-one"></div><div class="ambient ambient-two"></div>
    <section class="onboard-brand">
      <div class="fox-stage">
        <div class="orbit"><i></i><i></i><i></i></div>
        <img src="/fox.png" alt="Фиолетовый лис VipperFox" />
      </div>
      <div class="brand-copy">
        <span class="overline">TWITCH VIP COMPANION</span>
        <h1>Привет! Я <em>VipperFox</em></h1>
        <p>Прослежу за активностью VIP, поймаю Watch Streak и подготовлю аккуратный отчёт после стрима.</p>
        <div class="trust-row"><span><Icon name="shield" size={17}/> Токен хранится на этом ПК</span><span><Icon name="eye" size={17}/> Пароль Twitch не нужен</span></div>
      </div>
    </section>

    <section class="setup-card">
      <div class="steps">
        <button class:active={guideStep === 1} class:done={guideStep > 1} onclick={() => guideStep = 1}><b>{guideStep > 1 ? "✓" : "1"}</b><span>Получить токен<small>Открыть генератор</small></span></button>
        <i></i>
        <button class:active={guideStep === 2} onclick={() => guideStep = 2}><b>2</b><span>Подключить канал<small>Проверить права</small></span></button>
      </div>

      {#if guideStep === 1}
        <div class="guide-pane">
          <div class="guide-copy">
            <span class="section-kicker">ШАГ 1 ИЗ 2</span>
            <h2>Создайте токен с двумя правами</h2>
            <p>В генераторе выберите <strong>Custom Scope Token</strong>, войдите под аккаунтом владельца канала и отметьте только два разрешения ниже.</p>
            <div class="scope-list">
              <div><Icon name="check" size={16}/><span><strong>channel:manage:vips</strong><small>Чтение, выдача и снятие VIP</small></span></div>
              <div><Icon name="check" size={16}/><span><strong>user:read:chat</strong><small>Сообщения чата и Watch Streak</small></span></div>
            </div>
            <button class="primary-button" onclick={() => openExternal("https://twitchtokengenerator.com/")}>Открыть Token Generator <Icon name="external" size={17}/></button>
            {#if externalError}<div class="form-error"><Icon name="x" size={16}/>{externalError}</div>{/if}
            <button class="text-button" onclick={() => guideStep = 2}>У меня уже есть данные <Icon name="chevron" size={16}/></button>
          </div>
          <aside class="guide-note"><div class="note-fox"><img src="/fox.png" alt=""/></div><strong>Лисий совет</strong><p>Не отправляйте токен другим людям. VipperFox сохранит его в защищённом хранилище Windows.</p></aside>
        </div>
      {:else}
        <form class="connect-pane" onsubmit={(event) => { event.preventDefault(); connect(); }}>
          <span class="section-kicker">ШАГ 2 ИЗ 2</span>
          <h2>Подключите Twitch-канал</h2>
          <p>Вставьте Client ID и Access Token, полученные на предыдущем шаге.</p>
          <label><span>Client ID</span><input autocomplete="off" bind:value={clientId} placeholder="Например: abc123..." /></label>
          <label><span>Access Token</span><div class="secret-field"><input type={showToken ? "text" : "password"} autocomplete="off" bind:value={accessToken} placeholder="Токен не отображается"/><button type="button" onclick={() => showToken = !showToken}><Icon name="eye" size={18}/></button></div></label>
          {#if connectError}<div class="form-error"><Icon name="x" size={16}/>{connectError}</div>{/if}
          <div class="connect-actions"><button class="secondary-button" type="button" onclick={() => guideStep = 1}>Назад</button><button class="primary-button" disabled={connecting || !clientId.trim() || !accessToken.trim()}>{connecting ? "Проверяем…" : "Подключить канал"}<Icon name="chevron" size={17}/></button></div>
          <div class="privacy"><Icon name="shield" size={16}/> Данные проверяются напрямую через Twitch и не покидают ваш компьютер.</div>
        </form>
      {/if}
    </section>
  </main>
{:else}
  <div class="app-shell">
    <div class="titlebar" data-tauri-drag-region>
      <div class="title-brand" data-tauri-drag-region><img src="/fox.png" alt=""/><b>Vipper<span>Fox</span></b></div>
      <div class="window-controls"><button aria-label="Свернуть" onclick={() => !previewMode && getCurrentWindow().minimize()}>—</button><button class="close" aria-label="Закрыть" onclick={() => !previewMode && getCurrentWindow().close()}>×</button></div>
    </div>
    <aside class="sidebar">
      <div class="channel-chip"><div class="avatar">{snapshot.channelDisplayName?.slice(0, 1).toUpperCase()}</div><div><strong>{snapshot.channelDisplayName}</strong><span>@{snapshot.channelLogin}</span></div><i class:online={snapshot.connectionStatus === "connected"}></i></div>
      <nav>
        <button class:active={view === "dashboard"} onclick={() => view = "dashboard"}><Icon name="home"/><span>Обзор</span></button>
        <button class:active={view === "vips"} onclick={() => view = "vips"}><Icon name="users"/><span>VIP-зрители</span><b>{snapshot.vipCount}</b></button>
        <button class:active={view === "history"} onclick={() => view = "history"}><Icon name="history"/><span>История</span></button>
        <button class:active={view === "settings"} onclick={() => view = "settings"}><Icon name="settings"/><span>Настройки</span></button>
      </nav>
      <div class="sidebar-fox"><span class="spark s1">✦</span><span class="spark s2">✧</span><img src="/fox.png" alt="VipperFox"/><p>{snapshot.stream ? "Я слежу за чатом!" : "Жду следующий стрим"}</p></div>
      <div class="connection-line"><i class:online={snapshot.connectionStatus === "connected"}></i><span>{snapshot.connectionStatus === "connected" ? "EventSub подключён" : "Нет соединения"}</span></div>
    </aside>

    <main class="content">
      {#if view === "dashboard"}
        <header class="page-header"><div><span class="section-kicker">ДОБРЫЙ ВЕЧЕР</span><h1>Всё под контролем <span>✦</span></h1><p>VipperFox наблюдает за каналом и сохраняет только нужные события.</p></div><div class="header-actions"><button class="icon-button"><Icon name="history"/></button><div class="live-badge" class:offline={!snapshot.stream}><i></i>{snapshot.stream ? "LIVE" : "OFFLINE"}</div></div></header>

        {#if snapshot.stream && !snapshot.stream.complete}<div class="warning-banner"><div><Icon name="shield"/><span><strong>Мониторинг трансляции неполный</strong><small>Программа подключилась после начала стрима или теряла соединение. Массовое снятие будет требовать дополнительного подтверждения.</small></span></div></div>{/if}

        <section class="stat-grid">
          <article class="stat-card violet"><div class="stat-icon"><Icon name="users"/></div><div><span>Всего VIP</span><strong>{snapshot.vipCount}</strong><small>Актуальный список Twitch</small></div><div class="mini-orbit"></div></article>
          <article class="stat-card green"><div class="stat-icon"><Icon name="message"/></div><div><span>Писали сегодня</span><strong>{activeVips}</strong><small>{snapshot.vipCount ? Math.round(activeVips / snapshot.vipCount * 100) : 0}% VIP активны</small></div><div class="progress-ring" style={`--progress:${snapshot.vipCount ? activeVips / snapshot.vipCount * 360 : 0}deg`}><span>{snapshot.vipCount ? Math.round(activeVips / snapshot.vipCount * 100) : 0}%</span></div></article>
          <article class="stat-card amber"><div class="stat-icon"><Icon name="eye"/></div><div><span>Пока молчат</span><strong>{silentVips}</strong><small>Итог после завершения</small></div><div class="soft-dots">•••</div></article>
          <article class="stat-card blue"><div class="stat-icon"><Icon name="sparkles"/></div><div><span>Порог VIP</span><strong>{snapshot.streakThreshold}+</strong><small>Watch Streak</small></div><span class="auto-pill">AUTO</span></article>
        </section>

        <section class="dashboard-grid">
          <article class="panel live-panel"><div class="panel-head"><div><span class="section-kicker">ТЕКУЩАЯ ТРАНСЛЯЦИЯ</span><h2>{snapshot.stream ? "Мониторинг в реальном времени" : "Канал сейчас офлайн"}</h2></div>{#if snapshot.stream}<span class="monitor-pill"><i></i> Запись активности</span>{/if}</div>
            {#if snapshot.stream}
              <div class="stream-flow"><div class="flow-node"><Icon name="message"/><span><strong>Сообщения чата</strong><small>Каждое событие EventSub</small></span></div><i></i><div class="flow-node"><Icon name="users"/><span><strong>Сверка VIP</strong><small>По Twitch User ID</small></span></div><i></i><div class="flow-node"><Icon name="shield"/><span><strong>Безопасный отчёт</strong><small>После stream.offline</small></span></div></div>
              <div class="stream-meta"><span>Длительность <b>{streamDuration()}</b></span><span>Полнота <b class:good={snapshot.stream.complete}>{snapshot.stream.complete ? "100%" : "неполная"}</b></span><span>Получено сообщений <b>{activeVips || "—"}</b></span></div>
            {:else}<div class="offline-state"><div class="moon">☾</div><div><strong>Можно свернуть приложение в трей</strong><p>VipperFox автоматически начнёт мониторинг при следующем `stream.online`.</p></div></div>{/if}
          </article>

          <article class="panel activity-panel"><div class="panel-head"><div><span class="section-kicker">ПОСЛЕДНИЕ СОБЫТИЯ</span><h2>Лисья хроника</h2></div><button class="text-button" onclick={() => view = "history"}>Все события <Icon name="chevron" size={14}/></button></div>
            <div class="activity-list">{#each snapshot.activities.slice(0, 4) as activity, index}<div class="activity-item" style={`--delay:${index * 55}ms`}><div class:remove={activity.kind === "vip_remove"} class:warning={activity.kind === "warning"} class="activity-icon"><Icon name={activity.kind === "vip_add" ? "sparkles" : activity.kind === "vip_remove" ? "trash" : "shield"} size={17}/></div><div><strong>{activity.title}</strong><span>{activity.detail}</span></div><time>{timeLabel(activity.createdAt)}</time></div>{:else}<div class="empty-small"><Icon name="history"/><p>События появятся после подключения EventSub</p></div>{/each}</div>
          </article>
        </section>
      {:else if view === "vips"}
        <header class="page-header"><div><span class="section-kicker">УПРАВЛЕНИЕ</span><h1>VIP-зрители</h1><p>Список доступен и обновляется даже когда канал офлайн.</p></div><div class="header-actions"><button class="secondary-button" disabled={refreshingVips} onclick={reloadVips}>{refreshingVips ? "Обновляем…" : "Обновить список"}</button><div class="live-badge" class:offline={!snapshot.stream}><i></i>{snapshot.stream ? "МОНИТОРИНГ" : "ОЖИДАНИЕ"}</div></div></header>
        {#if refreshError}<div class="warning-banner"><div><Icon name="shield"/><span><strong>Не удалось обновить VIP</strong><small>{refreshError}</small></span></div></div>{/if}
        <section class="panel table-panel"><div class="panel-head"><div><h2>Текущий список</h2><span class="table-summary">{snapshot.stream ? `${activeVips} активны · ${silentVips} молчат` : "Канал офлайн · список доступен"}</span></div><div class="search-mock">⌕ <span>Поиск зрителя</span></div></div><div class="vip-table"><div class="table-row table-header"><span>Пользователь</span><span>Watch Streak</span><span>Текущий стрим</span><span>Статус</span></div>{#each snapshot.vips as vip, index}<div class="table-row" style={`--delay:${index * 40}ms`}><span class="user-cell"><b>{vip.displayName.slice(0,1).toUpperCase()}</b><span><strong>{vip.displayName}</strong><small>@{vip.login}</small></span></span><span>{vip.watchStreak ?? "—"}{#if vip.watchStreak && vip.watchStreak >= snapshot.streakThreshold}<em class="streak-star">✦</em>{/if}</span><span><span class:yes={snapshot.stream && vip.wroteThisStream} class="status-chip">{snapshot.stream ? (vip.wroteThisStream ? "✓ Писал" : "Ещё нет") : "Канал офлайн"}</span></span><span><span class="vip-badge">VIP</span></span></div>{/each}</div></section>
      {:else if view === "history"}
        <header class="page-header"><div><span class="section-kicker">ЖУРНАЛ</span><h1>История действий</h1><p>Выдача VIP, ручные снятия и состояние мониторинга.</p></div></header>
        <section class="panel history-panel">{#each snapshot.activities as activity}<div class="history-row"><div class:remove={activity.kind === "vip_remove"} class:warning={activity.kind === "warning"} class="activity-icon"><Icon name={activity.kind === "vip_add" ? "sparkles" : activity.kind === "vip_remove" ? "trash" : "shield"}/></div><div><strong>{activity.title}</strong><p>{activity.detail}</p></div><time>{new Intl.DateTimeFormat("ru", {day:"2-digit",month:"long",hour:"2-digit",minute:"2-digit"}).format(new Date(activity.createdAt))}</time></div>{:else}<div class="empty-large"><Icon name="history" size={34}/><h3>История пока пуста</h3><p>Первое событие появится после подключения Twitch.</p></div>{/each}</section>
      {:else}
        <header class="page-header"><div><span class="section-kicker">ПАРАМЕТРЫ</span><h1>Настройки</h1><p>Поведение автоматизации и подключение канала.</p></div></header>
        <section class="settings-grid"><article class="panel settings-card appearance-card"><div class="settings-icon"><Icon name="eye"/></div><div><h2>Размер и читаемость</h2><p>Настройки применяются сразу и сохраняются на этом компьютере.</p><div class="preset-setting"><span>Интерфейс</span><div class="preset-buttons"><button class:active={interfaceSize === "compact"} onclick={() => chooseInterfaceSize("compact")}>90%</button><button class:active={interfaceSize === "standard"} onclick={() => chooseInterfaceSize("standard")}>110%</button><button class:active={interfaceSize === "large"} onclick={() => chooseInterfaceSize("large")}>130%</button></div></div><div class="preset-setting"><span>Шрифт</span><div class="preset-buttons"><button class:active={fontSize === "normal"} onclick={() => chooseFontSize("normal")}>Обычный</button><button class:active={fontSize === "large"} onclick={() => chooseFontSize("large")}>Крупный</button><button class:active={fontSize === "xlarge"} onclick={() => chooseFontSize("xlarge")}>Очень крупный</button></div></div></div></article><article class="panel settings-card"><div class="settings-icon"><Icon name="sparkles"/></div><div><h2>Автоматическая выдача VIP</h2><p>VipperFox выдаёт VIP при официальном Watch Streak notification.</p><label class="number-setting"><span>Минимальная серия</span><div><input type="number" min="1" bind:value={settingsThreshold}/><b>стримов</b></div></label><button class="primary-button compact" onclick={saveSettings}>{settingsSaved ? "Сохранено ✓" : "Сохранить"}</button></div></article><article class="panel settings-card danger-zone"><div class="settings-icon"><Icon name="shield"/></div><div><h2>Подключённый канал</h2><p><strong>{snapshot.channelDisplayName}</strong> · @{snapshot.channelLogin}</p><button class="secondary-button danger" onclick={async () => snapshot = await disconnectTwitch()}>Отключить Twitch</button></div></article></section>
      {/if}

      {#if snapshot.report.length}
        <section class="report-drawer"><div class="report-head"><div><span class="section-kicker">СТРИМ ЗАВЕРШЁН</span><h2>{snapshot.report.length} VIP не писали в чат</h2><p>{snapshot.reportComplete ? "Проверьте список — снятие выполняется только вручную, пакетами до 20 человек." : "Данные неполные: снятие VIP заблокировано. Проверьте пользователей вручную в Twitch."}</p></div><button class="secondary-button" disabled={!snapshot.reportComplete || removing} onclick={selectAllReport}>{selected.size === Math.min(snapshot.report.length, 20) ? "Снять выделение" : "Выбрать до 20"}</button></div>{#if removalError}<div class="warning-banner"><div><Icon name="shield"/><span><strong>Операция не выполнена</strong><small>{removalError}</small></span></div></div>{/if}<div class="report-list">{#each snapshot.report as user}<label class:selected={selected.has(user.userId)} class:disabled={!snapshot.reportComplete}><input type="checkbox" disabled={!snapshot.reportComplete || removing} checked={selected.has(user.userId)} onchange={() => toggleSelected(user.userId)}/><span class="checkmark"><Icon name="check" size={14}/></span><span class="user-cell"><b>{user.displayName.slice(0,1).toUpperCase()}</b><span><strong>{user.displayName}</strong><small>@{user.login}</small></span></span><span class="status-chip">Не писал</span></label>{/each}</div><div class="report-actions"><span>Выбрано: <b>{selected.size}</b></span><button class="danger-button" disabled={!snapshot.reportComplete || !selected.size || removing} onclick={() => { removalConfirmation = ""; removalError = ""; confirmOpen = true; }}><Icon name="trash" size={17}/>{removing ? "Снимаем…" : `Снять VIP у выбранных (${selected.size})`}</button></div></section>
      {/if}
    </main>
  </div>
{/if}

{#if confirmOpen}
  <!-- svelte-ignore a11y_interactive_supports_focus a11y_click_events_have_key_events -->
  <div class="modal-backdrop" role="presentation" onclick={() => !removing && (confirmOpen = false)}><section class="confirm-modal" role="dialog" aria-modal="true" onclick={(e) => e.stopPropagation()}><div class="modal-icon"><Icon name="trash" size={25}/></div><h2>Снять VIP у {selected.size} {selected.size === 1 ? "пользователя" : "пользователей"}?</h2><p>Это изменит роли на Twitch. Для защиты введите логин подключённого канала: <strong>{snapshot.channelLogin}</strong></p><div class="selected-names">{selectedReport.slice(0,3).map((u: ReportUser) => u.displayName).join(", ")}{selected.size > 3 ? ` и ещё ${selected.size - 3}` : ""}</div><input class="danger-confirm-input" autocomplete="off" spellcheck="false" bind:value={removalConfirmation} placeholder={snapshot.channelLogin ?? "логин канала"}/>{#if removalError}<p class="modal-error">{removalError}</p>{/if}<div><button class="secondary-button" disabled={removing} onclick={() => confirmOpen = false}>Отмена</button><button class="danger-button" disabled={!confirmationMatches || removing} onclick={confirmRemoval}>{removing ? "Снимаем…" : "Да, снять VIP"}</button></div></section></div>
{/if}
