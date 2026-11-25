<!-- Every primitive of the Waffle system in each of its states, and the tokens. Development
     and tests only; never part of the app bundle. -->
<script lang="ts">
  import AppHeader from "../lib/components/primitives/AppHeader.svelte";
  import BalanceFigure from "../lib/components/primitives/BalanceFigure.svelte";
  import BigNumber from "../lib/components/primitives/BigNumber.svelte";
  import Checkbox from "../lib/components/primitives/Checkbox.svelte";
  import CommandLine from "../lib/components/primitives/CommandLine.svelte";
  import ConfirmDialog from "../lib/components/primitives/ConfirmDialog.svelte";
  import CopyButton from "../lib/components/primitives/CopyButton.svelte";
  import DataTable from "../lib/components/primitives/DataTable.svelte";
  import DetailRows from "../lib/components/primitives/DetailRows.svelte";
  import EmptyState from "../lib/components/primitives/EmptyState.svelte";
  import ErrorState from "../lib/components/primitives/ErrorState.svelte";
  import Freshness from "../lib/components/primitives/Freshness.svelte";
  import HistoryChart from "../lib/components/primitives/HistoryChart.svelte";
  import IconButton from "../lib/components/primitives/IconButton.svelte";
  import InlineBanner from "../lib/components/primitives/InlineBanner.svelte";
  import ListRow from "../lib/components/primitives/ListRow.svelte";
  import LoadingWaffle from "../lib/components/primitives/LoadingWaffle.svelte";
  import Menu from "../lib/components/primitives/Menu.svelte";
  import Meter from "../lib/components/primitives/Meter.svelte";
  import MeterLabel from "../lib/components/primitives/MeterLabel.svelte";
  import MeterRow from "../lib/components/primitives/MeterRow.svelte";
  import NoSource from "../lib/components/primitives/NoSource.svelte";
  import NumberField from "../lib/components/primitives/NumberField.svelte";
  import PlanTag from "../lib/components/primitives/PlanTag.svelte";
  import RadioGroup from "../lib/components/primitives/RadioGroup.svelte";
  import RangePills from "../lib/components/primitives/RangePills.svelte";
  import Section from "../lib/components/primitives/Section.svelte";
  import SelectField from "../lib/components/primitives/SelectField.svelte";
  import Sparkline from "../lib/components/primitives/Sparkline.svelte";
  import StaleRow from "../lib/components/primitives/StaleRow.svelte";
  import StatusBadge from "../lib/components/primitives/StatusBadge.svelte";
  import Switch from "../lib/components/primitives/Switch.svelte";
  import TextTabs from "../lib/components/primitives/TextTabs.svelte";
  import Toast from "../lib/components/primitives/Toast.svelte";
  import Tooltip from "../lib/components/primitives/Tooltip.svelte";
  import WarningLegend from "../lib/components/primitives/WarningLegend.svelte";
  import type {
    SortDirection,
    TableCell,
    TableColumn,
  } from "../lib/components/primitives/display";
  import type { WindowMeterView } from "../lib/format";
  import type { HistorySeries } from "../lib/ipc/bindings/HistorySeries";

  type Range = "day" | "week" | "month";

  const ranges: readonly { value: Range; label: string }[] = [
    { value: "day", label: "24 h" },
    { value: "week", label: "7 d" },
    { value: "month", label: "30 d" },
  ];

  const colours: readonly string[] = [
    "paper",
    "ink",
    "muted",
    "faint",
    "rule",
    "chip",
    "alert",
    "critical-fill",
    "stale",
    "ink-hover",
    "ink-pressed",
    "scrim",
    "hover",
    "pressed",
    "selected",
  ];

  const spaces: readonly number[] = [1, 2, 3, 4, 6, 8, 12, 14, 18];

  const nowS = Math.floor(Date.now() / 1000);

  const meter: WindowMeterView = {
    kind: "window",
    id: "window:five_hour",
    window: { kind: "five_hour" },
    used_bp: 8_600,
    resets_at: nowS + 8_040,
    status: "warning",
    severity: "warn",
    phase: "current",
  };

  const series: HistorySeries = {
    provider: "claude",
    meter_id: "window:five_hour",
    from: nowS - 86_400,
    to: nowS,
    unit: "basis_points",
    gap_s: 7_200,
    resets: [nowS - 43_200],
    points: [12, 20, 34, 51, 66, 82, 8, 17, 29, 40, 55, 61].map((pct, at) => ({
      t: nowS - 82_800 + at * 7_200,
      v: pct * 100,
      lo: pct * 100,
      hi: pct * 100,
    })),
  };

  function trace(
    percents: readonly number[],
    unit: HistorySeries["unit"] = "basis_points",
  ): HistorySeries {
    return {
      ...series,
      unit,
      points: percents.map((pct, at) => ({
        t: nowS - 43_200 + at * 3_600,
        v: pct * 100,
        lo: pct * 100,
        hi: pct * 100,
      })),
    };
  }

  const traces: readonly { name: string; series: HistorySeries | null }[] = [
    { name: "Normal", series: trace([22, 40, 31, 56, 49, 40, 52]) },
    { name: "Past warning", series: trace([22, 40, 31, 56, 72, 86, 93]) },
    { name: "Flat", series: trace([38, 38, 38]) },
    { name: "No data", series: null },
  ];

  const columns: readonly TableColumn[] = [
    { label: "Date", tone: "muted" },
    { label: "Provider", tone: "strong" },
    { label: "Window" },
    { label: "Peak %", numeric: true },
    { label: "Resets", numeric: true, tone: "muted" },
  ];
  const tableRows: readonly (readonly TableCell[])[] = [
    [
      { text: "Nov 21" },
      { text: "Claude Max" },
      { text: "5-hour" },
      { text: "62%" },
      { text: "in 2h 10m" },
    ],
    [
      { text: "Nov 20" },
      { text: "Codex" },
      { text: "Weekly" },
      { text: "83%", tone: "alert" },
      { text: "in 2d 1h" },
    ],
    [
      { text: "Nov 19" },
      { text: "DeepSeek" },
      { text: "Balance" },
      { text: "Timed out", tone: "muted" },
      { text: "Retry in 1m" },
    ],
  ];
  let sort = $state<{ index: number; direction: SortDirection }>({
    index: 3,
    direction: "ascending",
  });
  function setSort(index: number): void {
    sort = {
      index,
      direction:
        sort.index === index && sort.direction === "ascending"
          ? "descending"
          : "ascending",
    };
  }

  const nothing = (): void => undefined;

  let range = $state<Range>("day");
  let tab = $state<"usage" | "history">("usage");
  let choice = $state<Range>("week");
  let on = $state(true);
  let dialog = $state<"confirm" | "info" | null>(null);

  function setOn(next: boolean): void {
    on = next;
  }
  function setChoice(next: Range): void {
    choice = next;
  }
  function setRange(next: Range): void {
    range = next;
  }
  function setTab(next: "usage" | "history"): void {
    tab = next;
  }
</script>

<main>
  <h1>Component gallery</h1>

  <section data-gallery="tokens" aria-label="Tokens">
    <h2>Colour</h2>
    <ul class="swatches">
      {#each colours as name (name)}
        <li><span class="swatch {name}"></span>{name}</li>
      {/each}
    </ul>
    <h2>Type</h2>
    <p class="type title">Title, Instrument Serif 48/56</p>
    <p class="type number">86%</p>
    <p class="type dialog-title">Dialog title, 24/32</p>
    <p class="type body">Body and name, DM Sans 14/20</p>
    <p class="type caption">Caption and label, 13/20</p>
    <p class="type micro">Micro, 12/16</p>
    <p class="type label">Label, 13/20 medium</p>
    <p class="type mono">DM Mono: codex login</p>
    <h2>Space</h2>
    <ul class="spaces">
      {#each spaces as step (step)}
        <li><span class="bar space-{step}"></span>{step * 4}</li>
      {/each}
    </ul>
    <h2>Icons</h2>
    <div class="row">
      <IconButton label="Refresh icon" icon="refresh" quiet onclick={nothing} />
      <IconButton label="Close icon" icon="close" quiet onclick={nothing} />
      <IconButton label="Back icon" icon="back" quiet onclick={nothing} />
      <IconButton label="More icon" icon="more" quiet onclick={nothing} />
      <CopyButton text="codex login" />
      <StatusBadge kind="near_limit" />
      <StatusBadge kind="stale" />
      <StatusBadge kind="healthy" />
      <StatusBadge kind="no_source" />
      <WarningLegend />
    </div>
  </section>

  <section data-gallery="buttons" aria-label="Buttons">
    <h2>Buttons</h2>
    <div class="row">
      <button type="button" class="primary">Primary</button>
      <button type="button">Outline</button>
      <button type="button" class="text">Text</button>
      <IconButton label="Refresh" icon="refresh" onclick={nothing} />
      <IconButton label="More" icon="more" quiet onclick={nothing} />
    </div>
    <div class="row">
      <button type="button" class="primary small">Primary</button>
      <button type="button" class="small">Outline</button>
      <button type="button" class="text small">Text</button>
      <IconButton label="Close" icon="close" size="small" onclick={nothing} />
      <IconButton label="Back" icon="back" size="small" onclick={nothing} />
    </div>
    <div class="row">
      <button type="button" class="primary" disabled>Primary off</button>
      <button type="button" disabled>Outline off</button>
      <button type="button" class="text" disabled>Text off</button>
      <IconButton
        label="Refresh off"
        icon="refresh"
        disabled
        onclick={nothing}
      />
    </div>
  </section>

  <section data-gallery="switches" aria-label="Switches, checkboxes, radios">
    <h2>Switch</h2>
    <div>
      <Switch label="Off" checked={false} onchange={nothing} />
      <Switch label="On" checked={on} onchange={setOn} />
      <Switch
        label="Codex"
        hint="5-hour and weekly windows."
        strong
        checked
        onchange={nothing}
      />
      <Switch label="Disabled" checked={false} disabled onchange={nothing} />
    </div>
    <h2>Checkbox</h2>
    <Checkbox
      label="Unchecked. Claude Max"
      checked={false}
      onchange={nothing}
    />
    <Checkbox label="Checked. Codex" checked onchange={nothing} />
    <Checkbox
      label="Mixed. Some providers"
      checked={false}
      mixed
      onchange={nothing}
    />
    <Checkbox
      label="Warn at 80%"
      hint="Tell me when a window passes the warning mark."
      checked
      onchange={nothing}
    />
    <Checkbox label="Disabled" checked={false} disabled onchange={nothing} />
    <h2>Radio</h2>
    <RadioGroup
      legend="Window"
      options={ranges}
      value={choice}
      onchange={setChoice}
    />
    <RadioGroup
      legend="Disabled group"
      options={ranges}
      value="day"
      disabled
      onchange={nothing}
    />
  </section>

  <section data-gallery="inputs" aria-label="Inputs">
    <h2>Inputs</h2>
    <NumberField
      id="g-warn"
      label="Warning (%)"
      value={80}
      help="50 to 95."
      onchange={nothing}
    />
    <NumberField
      id="g-saved"
      label="Saved field"
      value={95}
      saved
      onchange={nothing}
    />
    <NumberField
      id="g-error"
      label="Critical (%)"
      value={70}
      error="Warn must be below critical (95)"
      onchange={nothing}
    />
    <NumberField
      id="g-off"
      label="Disabled field"
      value={80}
      disabled
      onchange={nothing}
    />
    <SelectField
      id="g-select"
      label="Theme"
      value={choice}
      options={ranges}
      onchange={nothing}
    />
    <h2>Command chip</h2>
    <CommandLine command="codex login" />
    <CommandLine command="DEEPSEEK_API_KEY=your-key-goes-here-and-it-is-long" />
  </section>

  <section data-gallery="navigation" aria-label="Pills, tabs, header">
    <h2>Range switch</h2>
    <RangePills
      legend="Range"
      options={ranges}
      value={range}
      onchange={setRange}
    />
    <h2>Segmented</h2>
    <RangePills
      legend="Providers"
      track
      options={[
        { value: "day", label: "All", count: 6 },
        { value: "week", label: "Past mark", count: 1 },
        { value: "month", label: "Stale", count: 0 },
      ]}
      value={range}
      onchange={setRange}
    />
    <RangePills
      legend="Disabled range"
      disabled
      options={ranges}
      value="day"
      onchange={nothing}
    />
    <h2>Text tabs</h2>
    <TextTabs
      label="Sections"
      tabs={[
        { value: "usage", label: "Usage" },
        { value: "history", label: "History" },
      ]}
      value={tab}
      onchange={setTab}
    >
      {#snippet children(current)}<p>Panel: {current}</p>{/snippet}
    </TextTabs>
    <h2>Header</h2>
    <AppHeader>
      <Freshness atMs={Date.now() - 12_000} />
      <RangePills
        legend="Header range"
        options={ranges}
        value="day"
        onchange={nothing}
      />
    </AppHeader>
  </section>

  <section data-gallery="overlays" aria-label="Tooltip, menu, dialogs">
    <h2>Tooltip and menu</h2>
    <div class="row tall">
      <Tooltip text="Resets today at 6:40 PM">
        {#snippet children(id)}<button type="button" aria-describedby={id}
            >Hover or focus</button
          >{/snippet}
      </Tooltip>
      <Menu
        label="Provider menu"
        items={[
          { label: "Refresh", shortcut: "R", onselect: nothing },
          { label: "Copy command", shortcut: "C", onselect: nothing },
          {
            label: "Settings",
            shortcut: ",",
            separated: true,
            onselect: nothing,
          },
          {
            label: "Hide provider",
            shortcut: "H",
            destructive: true,
            separated: true,
            onselect: nothing,
          },
        ]}
      />
    </div>
    <h2>Dialogs</h2>
    <div class="row">
      <button type="button" onclick={() => (dialog = "confirm")}
        >Open confirm</button
      >
      <button type="button" onclick={() => (dialog = "info")}>Open info</button>
    </div>
    {#if dialog !== null}
      <ConfirmDialog
        title={dialog === "info"
          ? "About the live check"
          : "Turn on the live check?"}
        body="Flashboard will send one small request to Claude each time it checks."
        confirmLabel={dialog === "info" ? "Got it" : "Turn on"}
        cancelLabel={dialog === "info" ? null : "Cancel"}
        onconfirm={() => (dialog = null)}
        oncancel={() => (dialog = null)}
      />
    {/if}
  </section>

  <section data-gallery="waffle" aria-label="Waffle meters">
    <h2>Waffle sizes and levels</h2>
    <div class="row top">
      <Meter
        usedBp={3_700}
        severity="ok"
        label="Large, normal"
        valueText="37 percent used"
      />
      <Meter
        usedBp={8_600}
        severity="warn"
        label="Medium, warning"
        valueText="86 percent used"
        size="medium"
      />
      <Meter
        usedBp={9_700}
        severity="critical"
        label="Small, critical"
        valueText="97 percent used"
        size="small"
      />
      <Meter
        usedBp={5_000}
        severity="ok"
        label="Mini"
        valueText="50 percent used"
        size="mini"
      />
      <Meter
        usedBp={6_100}
        severity="ok"
        label="Stale"
        valueText="61 percent used, stale"
        size="medium"
        stale
      />
    </div>
    <h2>Meter row</h2>
    <div class="row top">
      <MeterRow
        provider="claude"
        {meter}
        observedAt={Date.now() - 60_000}
        heading="Claude"
      />
      <MeterRow
        provider="codex"
        meter={{ ...meter, used_bp: 2_300, severity: "ok", status: "allowed" }}
        observedAt={Date.now() - 60_000}
        size="medium"
      />
    </div>
    <MeterLabel name="Claude" window="5-hour" status="near_limit" />
    <BigNumber value="37" unit="used" />
    <BigNumber value="86" unit="used" alert />
    <LoadingWaffle size="medium" announcement="" />
  </section>

  <section data-gallery="status" aria-label="Status and states">
    <h2>Badges and tags</h2>
    <div class="row">
      <StatusBadge kind="near_limit" />
      <StatusBadge kind="stale" />
      <StatusBadge kind="healthy" />
      <StatusBadge kind="no_source" />
      <PlanTag plan="Max 5x" />
      <WarningLegend />
      <Freshness atMs={Date.now() - 600_000} />
    </div>
    <StaleRow text="Showing data from 12 minutes ago" />
    <ErrorState staleText="Last seen 12 min ago" message="Can't reach Codex.">
      {#snippet command()}<CommandLine command="codex login" />{/snippet}
      {#snippet action()}<button type="button">Retry</button>{/snippet}
    </ErrorState>
    <InlineBanner
      title="Live check is off"
      body="Turn it on to read Claude between requests."
    >
      {#snippet action()}<button type="button">Turn on</button>{/snippet}
    </InlineBanner>
    <div class="stage">
      <Toast message="Copied" ondismiss={nothing} durationMs={600_000} />
    </div>
    <div class="stage">
      <Toast
        message="Autostart could not be written"
        warning
        ondismiss={nothing}
        durationMs={600_000}
      />
    </div>
    <EmptyState
      title="No usage yet"
      body="Make a request and it shows here."
      ghost
    />
    <NoSource names={["Gemini", "Antigravity", "Grok"]} />
  </section>

  <section data-gallery="data" aria-label="Lists, rows, history">
    <h2>List rows and sections</h2>
    <Section label="Providers">
      <ListRow
        name="Claude"
        window="5-hour"
        usedBp={8_600}
        severity="warn"
        onclick={nothing}
      />
      <ListRow
        name="Codex"
        window="7-day"
        usedBp={2_300}
        severity="ok"
        selected
        onclick={nothing}
      />
      <ListRow name="Grok" usedBp={null} onclick={nothing} />
    </Section>
    <Section label="Balance" secondary>
      <BalanceFigure amount="¥110.00" caption="DeepSeek balance" />
      <BalanceFigure amount="¥1.20" caption="Running low" low />
      <DetailRows
        rows={[
          { label: "Used", value: "86.0 %", alert: true },
          { label: "Status", value: "Warning" },
        ]}
      />
    </Section>
    <h2>History</h2>
    <HistoryChart
      {series}
      range="day"
      name="Claude 5-hour"
      thresholds={{ warnBp: 8_000, criticalBp: 9_500 }}
      currency={null}
    />
  </section>

  <section data-gallery="table" aria-label="Table">
    <h2>Table</h2>
    <DataTable
      caption="Usage history"
      {columns}
      rows={tableRows}
      sortIndex={sort.index}
      sortDirection={sort.direction}
      onsort={setSort}
    />
  </section>

  <section data-gallery="sparkline" aria-label="Sparkline">
    <h2>Sparkline</h2>
    <ul class="swatches">
      {#each traces as item (item.name)}
        <li><Sparkline series={item.series} />{item.name}</li>
      {/each}
    </ul>
    {#each traces as item (item.name)}
      <ListRow
        name={item.name}
        window="5-hour"
        usedBp={item.series === null ? null : 6_200}
        note="No data"
        onclick={nothing}
      >
        {#snippet trace()}<Sparkline series={item.series} />{/snippet}
      </ListRow>
    {/each}
  </section>

  <section data-gallery="dot-tooltip" aria-label="Dot tooltip">
    <h2>Waffle with dot names</h2>
    <div class="tall">
      <Meter
        usedBp={6_200}
        severity="ok"
        label="Claude 5-hour with dot names"
        focusable
        dotTooltip
      />
    </div>
  </section>
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    gap: var(--space-8);
    max-width: 48rem;
    margin-inline: auto;
    padding: var(--space-4);
  }

  h1 {
    font-family: var(--font-serif);
    font-size: var(--title-xl);
    font-weight: var(--title-xl-weight);
    line-height: var(--title-xl-line);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-block: var(--space-4);
    border-top: var(--rule) solid var(--color-rule);
    background: var(--color-paper);
  }

  h2 {
    font-size: var(--text-md);
    font-weight: var(--text-md-weight-strong);
    line-height: var(--text-md-line);
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-4);
  }

  .top {
    align-items: flex-start;
  }

  /* A transform makes this the containing block of the toast's fixed position. */
  .stage {
    position: relative;
    min-height: var(--space-18);
    translate: 0;
    transform: translateZ(0);
  }

  .tall {
    align-items: flex-start;
    min-height: 15rem;
    padding-top: var(--space-12);
  }

  .swatches {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(9rem, 1fr));
    gap: var(--space-3);
    margin: 0;
    padding: 0;
    list-style: none;
    font-family: var(--font-mono);
    font-size: var(--text-micro);
  }

  li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .swatch {
    flex-shrink: 0;
    width: var(--space-8);
    height: var(--space-8);
    border: var(--outline) solid var(--color-faint);
    border-radius: var(--radius-small);
  }

  .paper {
    background: var(--color-paper);
  }
  .ink {
    background: var(--color-ink);
  }
  .muted {
    background: var(--color-muted);
  }
  .faint {
    background: var(--color-faint);
  }
  .rule {
    background: var(--color-rule);
  }
  .chip {
    background: var(--color-chip);
  }
  .alert {
    background: var(--color-alert);
  }
  .critical-fill {
    background: var(--color-critical-fill);
  }
  .stale {
    background: var(--color-stale);
  }
  .ink-hover {
    background: var(--color-ink-hover);
  }
  .ink-pressed {
    background: var(--color-ink-pressed);
  }
  .scrim {
    background: var(--color-scrim);
  }
  .hover {
    background: var(--color-hover);
  }
  .pressed {
    background: var(--color-pressed);
  }
  .selected {
    background: var(--color-selected);
  }

  .spaces {
    display: grid;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-micro);
  }
  .bar {
    height: var(--space-3);
    background: var(--color-ink);
  }
  .space-1 {
    width: var(--space-1);
  }
  .space-2 {
    width: var(--space-2);
  }
  .space-3 {
    width: var(--space-3);
  }
  .space-4 {
    width: var(--space-4);
  }
  .space-6 {
    width: var(--space-6);
  }
  .space-8 {
    width: var(--space-8);
  }
  .space-12 {
    width: var(--space-12);
  }
  .space-14 {
    width: var(--space-14);
  }
  .space-18 {
    width: var(--space-18);
  }
  .label {
    font-size: var(--text-sm);
    font-weight: var(--text-sm-weight-strong);
    line-height: var(--text-sm-line);
  }

  .type {
    color: var(--color-text);
  }
  .title {
    font-family: var(--font-serif);
    font-size: var(--title-xl);
    line-height: var(--title-xl-line);
  }
  .number {
    font-family: var(--font-serif);
    font-size: var(--num-xl);
    line-height: var(--num-xl-line);
  }
  .dialog-title {
    font-family: var(--font-serif);
    font-size: var(--text-title);
    line-height: var(--text-title-line);
  }
  .body {
    font-size: var(--text-md);
    line-height: var(--text-md-line);
  }
  .caption {
    font-size: var(--text-sm);
    line-height: var(--text-sm-line);
  }
  .micro {
    font-size: var(--text-micro);
    line-height: var(--text-micro-line);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
