#!/usr/bin/env python3
"""
Generate RocksDB db_bench–specific plots from ext_stats CSV files.

These CSVs contain RocksDB internal metrics (write stalls, compaction stats,
write amplification) captured as time-series during saturator experiments.

Usage:
    python plot_db_bench.py                                    # All ext_stats in ./completed_output
    python plot_db_bench.py completed_output/4_14/             # All ext_stats under a directory
    python plot_db_bench.py path/to/ext_stats_saturation.csv   # A specific file
"""

import glob
import os
import sys
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
from matplotlib.ticker import MaxNLocator

from plotting.common import (
    C_BLUE, C_CYAN, C_GREEN, C_ORANGE, C_PURPLE, C_RED,
    ext_subdir, format_number, save_fig,
)


SOI_COLORS = {
    'cpu': C_BLUE, 'l1d': C_CYAN, 'l2': C_PURPLE, 'l3': C_GREEN,
    'membw': C_ORANGE, 'memcap': '#999999', 'iobw': C_RED, 'iops': '#8B4513',
}
SOI_MARKERS = {
    'cpu': 'o', 'l1d': 's', 'l2': '^', 'l3': 'D',
    'membw': 'v', 'memcap': 'x', 'iobw': 'P', 'iops': '*',
}


def _pivot_stats(df, group_col):
    """Pivot long-format ext_stats into wide-format with one row per
    (group, sample_idx, timestamp_ms) and one column per metric."""
    wide = df.pivot_table(
        index=[group_col, 'sample_idx', 'timestamp_ms'],
        columns='metric', values='value',
    ).reset_index()
    wide.columns.name = None
    return wide


def _add_elapsed(wide, group_col, max_elapsed_s=None):
    """Add an elapsed_s column relative to the first timestamp in each
    (group, sample_idx) combination. Optionally clip rows beyond max_elapsed_s
    to discard data from runs that spanned multiple measurement windows."""
    wide = wide.sort_values([group_col, 'sample_idx', 'timestamp_ms'])
    t0 = wide.groupby([group_col, 'sample_idx'])['timestamp_ms'].transform('min')
    wide['elapsed_s'] = (wide['timestamp_ms'] - t0) / 1000.0
    if max_elapsed_s is not None:
        wide = wide[wide['elapsed_s'] <= max_elapsed_s].copy()
    return wide


def _load_throughput_timeseries(stats_path, group_col):
    """Try to find and load the matching throughput timeseries CSV next to the
    ext_stats file. Returns None if not found."""
    folder = Path(stats_path).parent
    name = Path(stats_path).stem
    if 'saturation' in name:
        ts_name = 'timeseries_ext_saturation.csv'
    else:
        soi_type = name.replace('ext_stats_', '')
        ts_name = f'timeseries_ext_soi_{soi_type}.csv'
    ts_path = folder / ts_name
    if ts_path.exists():
        return pd.read_csv(ts_path)
    return None


def _parse_params(stats_path):
    """Read warmup_secs and duration_secs from the params.txt next to the CSV."""
    params_path = Path(stats_path).parent / 'params.txt'
    warmup = 30
    duration = 150
    if not params_path.exists():
        return warmup, duration
    for line in params_path.read_text().splitlines():
        try:
            if line.startswith('warmup_secs:'):
                warmup = int(line.split(':')[1].strip())
            elif line.startswith('duration_secs:'):
                duration = int(line.split(':')[1].strip())
        except ValueError:
            pass
    return warmup, duration


def _small_multiples(wide, group_col, group_label, metric, ylabel, folder,
                     title_prefix, out_name, warmup_s=30, derivative=False,
                     deriv_scale=1.0):
    """Render a small-multiples grid: one subplot per group level.
    If derivative=True, plot the rate of change instead of the raw value."""
    if metric not in wide.columns:
        return

    out = ext_subdir(folder, 'db_bench')
    groups = sorted(wide[group_col].unique())
    n = len(groups)
    if n == 0:
        return

    cols = min(n, 3)
    rows = (n + cols - 1) // cols
    fig, axes = plt.subplots(rows, cols, figsize=(6 * cols, 3.5 * rows),
                              squeeze=False, layout='constrained')

    for i, g in enumerate(groups):
        r, c = divmod(i, cols)
        ax = axes[r][c]

        sub = wide[wide[group_col] == g]
        agg = sub.groupby('elapsed_s')[metric].median().reset_index()
        agg = agg.sort_values('elapsed_s').dropna(subset=[metric])

        if derivative and len(agg) >= 2:
            dt = agg['elapsed_s'].diff()
            dv = agg[metric].diff()
            rate = (dv / dt * deriv_scale).fillna(0)
            ax.plot(agg['elapsed_s'].iloc[1:], rate.iloc[1:], '-', color=C_BLUE,
                    linewidth=1.3)
        elif not derivative:
            ax.plot(agg['elapsed_s'], agg[metric], '-', color=C_BLUE, linewidth=1.3)

        ax.axvline(x=warmup_s, color=C_RED, linestyle='--', alpha=0.5, linewidth=1)
        ax.set_title(f'{group_label}={int(g)}', fontsize=9)
        ax.set_xlabel('Elapsed (s)', fontsize=8)
        ax.set_ylabel(ylabel, fontsize=8)
        ax.tick_params(labelsize=7)
        ax.set_ylim(bottom=0)
        ax.grid(True, alpha=0.2)

    for j in range(i + 1, rows * cols):
        r, c = divmod(j, cols)
        axes[r][c].set_visible(False)

    fig.suptitle(f'{title_prefix} — {ylabel}', fontsize=11)
    save_fig(fig, os.path.join(out, out_name))


def plot_stall_timeseries(wide, group_col, group_label, folder, title_prefix, warmup_s=30):
    """Small-multiples of interval_stall_pct for each group level."""
    _small_multiples(wide, group_col, group_label, 'interval_stall_pct',
                     'Write Stall (%)', folder, title_prefix,
                     'stall_timeseries.png', warmup_s=warmup_s)


def plot_stall_heatmap(wide, group_col, group_label, folder, title_prefix, warmup_s=30):
    """Heatmap of interval_stall_pct: group on y-axis, time on x-axis.
    Resamples to a regular 1-second grid so rows align across groups."""
    if 'interval_stall_pct' not in wide.columns:
        return

    out = ext_subdir(folder, 'db_bench')
    groups = sorted(wide[group_col].unique())
    if len(groups) < 2:
        return

    max_t = wide['elapsed_s'].max()
    time_bins = np.arange(0, max_t + 1, 1.0)

    grid = []
    for g in groups:
        sub = wide[wide[group_col] == g].groupby('elapsed_s')['interval_stall_pct'].median().reset_index()
        sub = sub.sort_values('elapsed_s').dropna()
        if sub.empty:
            grid.append(np.full(len(time_bins), np.nan))
            continue
        resampled = np.interp(time_bins, sub['elapsed_s'].values,
                              sub['interval_stall_pct'].values,
                              left=np.nan, right=np.nan)
        grid.append(resampled)
    grid = np.array(grid)

    fig, ax = plt.subplots(figsize=(12, max(4, len(groups) * 0.6 + 1.5)))
    im = ax.imshow(grid, aspect='auto', cmap='YlOrRd', vmin=0, vmax=100,
                   extent=[0, max_t, len(groups) - 0.5, -0.5],
                   interpolation='nearest')
    ax.axvline(x=warmup_s, color='white', linestyle='--', alpha=0.8, linewidth=1.5)
    ax.set_xlabel('Elapsed (seconds)')
    ax.set_ylabel(group_label)
    ax.set_title(f'{title_prefix} — Write Stall Heatmap')
    ax.set_yticks(range(len(groups)))
    ax.set_yticklabels([str(int(g)) for g in groups])
    cbar = fig.colorbar(im, ax=ax, label='Interval Stall %')
    save_fig(fig, os.path.join(out, 'stall_heatmap.png'))


def plot_write_amp_vs_group(wide, group_col, group_label, folder, title_prefix):
    """Write amplification: final value per sample, box/whisker per group level."""
    if 'write_amp' not in wide.columns:
        return

    out = ext_subdir(folder, 'db_bench')
    groups = sorted(wide[group_col].unique())

    final_wa = []
    for g in groups:
        for s in wide['sample_idx'].unique():
            sub = wide[(wide[group_col] == g) & (wide['sample_idx'] == s)]
            sub = sub.dropna(subset=['write_amp']).sort_values('elapsed_s')
            if not sub.empty:
                final_wa.append({group_col: g, 'sample_idx': s,
                                 'write_amp': sub['write_amp'].iloc[-1]})
    if not final_wa:
        return
    wa_df = pd.DataFrame(final_wa)

    fig, ax = plt.subplots(figsize=(8, 5))
    medians = wa_df.groupby(group_col)['write_amp'].median()
    data = [wa_df[wa_df[group_col] == g]['write_amp'].values for g in groups]
    bp = ax.boxplot(data, positions=groups, widths=0.6, patch_artist=True,
                    boxprops=dict(facecolor=C_BLUE, alpha=0.3),
                    medianprops=dict(color=C_RED, linewidth=2))
    ax.plot(groups, [medians.get(g, 0) for g in groups], '-o', color=C_RED,
            linewidth=2, markersize=6, zorder=5)
    ax.set_xlabel(group_label)
    ax.set_ylabel('Write Amplification')
    ax.set_title(f'{title_prefix} — Final Write Amplification')
    ax.set_ylim(bottom=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(out, 'write_amp.png'))


def plot_compaction_rate(wide, group_col, group_label, folder, title_prefix, warmup_s=30):
    """Small-multiples of compaction rate (compactions/sec) derived from
    cumulative_compact_count."""
    _small_multiples(wide, group_col, group_label, 'cumulative_compact_count',
                     'Compactions / sec', folder, title_prefix,
                     'compaction_rate.png', warmup_s=warmup_s,
                     derivative=True)


def plot_compaction_write_rate(wide, group_col, group_label, folder, title_prefix, warmup_s=30):
    """Small-multiples of compaction write throughput (MB/s) derived from
    cumulative_compact_write_gb."""
    _small_multiples(wide, group_col, group_label, 'cumulative_compact_write_gb',
                     'Compaction Write (MB/s)', folder, title_prefix,
                     'compaction_write_rate.png', warmup_s=warmup_s,
                     derivative=True, deriv_scale=1024.0)


def plot_stall_vs_throughput(wide, ts_df, group_col, group_label, folder, title_prefix, warmup_s=30):
    """Scatter of interval_stall_pct vs ext_ops_sec from the throughput timeseries,
    joined on elapsed time. ext_stats elapsed includes warmup; throughput timeseries
    starts at measurement, so we offset ext_stats by warmup_s before joining."""
    if ts_df is None or 'interval_stall_pct' not in wide.columns:
        return
    if 'ext_ops_sec' not in ts_df.columns:
        return

    out = ext_subdir(folder, 'db_bench')
    groups = sorted(wide[group_col].unique())

    ts_col = group_col
    if ts_col not in ts_df.columns:
        return

    fig, ax = plt.subplots(figsize=(8, 6))
    cmap = plt.cm.viridis
    for i, g in enumerate(groups):
        stall_sub = wide[wide[group_col] == g].copy()
        stall_sub['meas_s'] = stall_sub['elapsed_s'] - warmup_s
        stall_agg = stall_sub[stall_sub['meas_s'] > 0].groupby('meas_s')['interval_stall_pct'].median().reset_index()

        ts_sub = ts_df[ts_df[ts_col] == g].copy()
        ts_sub['meas_s'] = ts_sub['elapsed_ms'] / 1000.0
        ts_agg = ts_sub.groupby('meas_s')['ext_ops_sec'].median().reset_index()

        merged = pd.merge_asof(
            stall_agg.sort_values('meas_s'),
            ts_agg.sort_values('meas_s'),
            on='meas_s', tolerance=1.5, direction='nearest',
        ).dropna(subset=['ext_ops_sec'])
        if merged.empty:
            continue
        color = cmap(i / max(len(groups) - 1, 1))
        ax.scatter(merged['interval_stall_pct'], merged['ext_ops_sec'],
                   s=20, alpha=0.5, color=color, label=f'{group_label}={int(g)}')
    ax.set_xlabel('Interval Write Stall (%)')
    ax.set_ylabel('External Throughput (ops/sec)')
    ax.set_title(f'{title_prefix} — Write Stall vs Throughput')
    ax.set_xlim(0, 105)
    ax.set_ylim(bottom=0)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    ax.legend(loc='best', fontsize=8, ncol=2)
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(out, 'stall_vs_throughput.png'))


def plot_aggregate_summary(wide, group_col, group_label, folder, title_prefix):
    """Multi-panel summary: median stall%, write amp, and compaction count
    aggregated per group level."""
    out = ext_subdir(folder, 'db_bench')
    groups = sorted(wide[group_col].unique())

    has_stall = 'cumulative_stall_pct' in wide.columns
    has_wa = 'write_amp' in wide.columns
    has_compact = 'cumulative_compact_count' in wide.columns

    panels = sum([has_stall, has_wa, has_compact])
    if panels == 0:
        return

    stats = []
    for g in groups:
        sub = wide[wide[group_col] == g]
        row = {group_col: g}
        for s in sub['sample_idx'].unique():
            ss = sub[sub['sample_idx'] == s].sort_values('elapsed_s')
            if has_stall:
                vals = ss['cumulative_stall_pct'].dropna()
                if len(vals):
                    row.setdefault('stall_vals', []).append(vals.iloc[-1])
            if has_wa:
                vals = ss['write_amp'].dropna()
                if len(vals):
                    row.setdefault('wa_vals', []).append(vals.iloc[-1])
            if has_compact:
                vals = ss['cumulative_compact_count'].dropna()
                if len(vals):
                    row.setdefault('compact_vals', []).append(vals.iloc[-1])
        if has_stall:
            sv = row.get('stall_vals', [])
            row['stall_median'] = np.median(sv) if sv else np.nan
        if has_wa:
            wv = row.get('wa_vals', [])
            row['wa_median'] = np.median(wv) if wv else np.nan
        if has_compact:
            cv = row.get('compact_vals', [])
            row['compact_median'] = np.median(cv) if cv else np.nan
        stats.append(row)
    sdf = pd.DataFrame(stats).dropna(subset=[c for c in ['stall_median', 'wa_median', 'compact_median']
                                              if c in pd.DataFrame(stats).columns], how='all')
    if sdf.empty:
        return

    fig, axes = plt.subplots(panels, 1, figsize=(9, 4 * panels), sharex=True,
                              layout='constrained')
    if panels == 1:
        axes = [axes]
    idx = 0

    if has_stall and sdf['stall_median'].notna().any():
        axes[idx].bar(sdf[group_col], sdf['stall_median'].fillna(0), color=C_RED, alpha=0.7,
                      edgecolor='black', linewidth=0.5)
        axes[idx].set_ylabel('Cumulative Stall %')
        axes[idx].set_title(f'{title_prefix} — RocksDB Summary')
        axes[idx].set_ylim(0, max(sdf['stall_median'].max() * 1.2, 10))
        axes[idx].grid(True, alpha=0.3, axis='y')
        for _, row in sdf.iterrows():
            if pd.notna(row['stall_median']):
                axes[idx].text(row[group_col], row['stall_median'] + 0.3,
                               f"{row['stall_median']:.1f}%", ha='center', va='bottom', fontsize=8)
        idx += 1

    if has_wa and sdf['wa_median'].notna().any():
        axes[idx].bar(sdf[group_col], sdf['wa_median'].fillna(0), color=C_ORANGE, alpha=0.7,
                      edgecolor='black', linewidth=0.5)
        axes[idx].set_ylabel('Write Amplification')
        axes[idx].set_ylim(0, max(sdf['wa_median'].max() * 1.2, 2))
        axes[idx].grid(True, alpha=0.3, axis='y')
        if idx == 0:
            axes[idx].set_title(f'{title_prefix} — RocksDB Summary')
        for _, row in sdf.iterrows():
            if pd.notna(row['wa_median']):
                axes[idx].text(row[group_col], row['wa_median'] + 0.05,
                               f"{row['wa_median']:.1f}x", ha='center', va='bottom', fontsize=8)
        idx += 1

    if has_compact and sdf['compact_median'].notna().any():
        axes[idx].bar(sdf[group_col], sdf['compact_median'].fillna(0), color=C_BLUE, alpha=0.7,
                      edgecolor='black', linewidth=0.5)
        axes[idx].set_ylabel('Total Compactions')
        axes[idx].set_ylim(0, max(sdf['compact_median'].max() * 1.2, 5))
        axes[idx].grid(True, alpha=0.3, axis='y')
        if idx == 0:
            axes[idx].set_title(f'{title_prefix} — RocksDB Summary')
        idx += 1

    axes[-1].set_xlabel(group_label)
    axes[-1].xaxis.set_major_locator(MaxNLocator(integer=True))
    save_fig(fig, os.path.join(out, 'rocksdb_summary.png'))


def plot_stall_throughput_dual(wide, ts_df, group_col, group_label, folder, title_prefix, warmup_s=30):
    """Dual-axis time-series: throughput on left, interval stall% on right,
    one subplot per group level. Both series are plotted on a shared
    measurement-time x-axis (ext_stats is shifted by warmup_s)."""
    if ts_df is None or 'interval_stall_pct' not in wide.columns:
        return
    if 'ext_ops_sec' not in ts_df.columns or group_col not in ts_df.columns:
        return

    out = ext_subdir(folder, 'db_bench')
    groups = sorted(wide[group_col].unique())
    n = len(groups)
    if n == 0:
        return

    cols = min(n, 3)
    rows = (n + cols - 1) // cols
    fig, axes = plt.subplots(rows, cols, figsize=(6 * cols, 4 * rows),
                              squeeze=False, layout='constrained')

    for i, g in enumerate(groups):
        r, c = divmod(i, cols)
        ax_tp = axes[r][c]

        stall_sub = wide[wide[group_col] == g].copy()
        stall_sub['meas_s'] = stall_sub['elapsed_s'] - warmup_s
        stall_agg = stall_sub[stall_sub['meas_s'] >= 0].groupby('meas_s')['interval_stall_pct'].median().reset_index()

        ts_sub = ts_df[ts_df[group_col] == g].copy()
        ts_sub['meas_s'] = ts_sub['elapsed_ms'] / 1000.0
        ts_agg = ts_sub.groupby('meas_s')['ext_ops_sec'].median().reset_index()

        l1, = ax_tp.plot(ts_agg['meas_s'], ts_agg['ext_ops_sec'], '-', color=C_BLUE,
                         linewidth=1.5, label='Throughput')
        ax_tp.set_ylabel('ops/sec', color=C_BLUE, fontsize=8)
        ax_tp.tick_params(axis='y', labelcolor=C_BLUE, labelsize=7)
        ax_tp.set_ylim(bottom=0)
        ax_tp.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

        ax_st = ax_tp.twinx()
        l2, = ax_st.plot(stall_agg['meas_s'], stall_agg['interval_stall_pct'], '-',
                         color=C_RED, linewidth=1.2, alpha=0.7, label='Stall %')
        ax_st.set_ylim(0, 105)
        ax_st.set_ylabel('Stall %', color=C_RED, fontsize=8)
        ax_st.tick_params(axis='y', labelcolor=C_RED, labelsize=7)

        ax_tp.set_title(f'{group_label}={int(g)}', fontsize=9)
        ax_tp.set_xlabel('Measurement time (s)', fontsize=8)
        ax_tp.grid(True, alpha=0.2)
        if i == 0:
            ax_tp.legend([l1, l2], ['Throughput', 'Stall %'], loc='upper right', fontsize=7)

    for j in range(i + 1, rows * cols):
        r, c = divmod(j, cols)
        axes[r][c].set_visible(False)

    fig.suptitle(f'{title_prefix} — Throughput vs Write Stall', fontsize=11)
    save_fig(fig, os.path.join(out, 'stall_throughput_panels.png'))


def plot_soi_comparison_rocksdb(ext_stats_csvs):
    """Cross-SoI comparison of RocksDB metrics: final stall% and write amp
    per SoI type, as a grouped bar chart."""
    if len(ext_stats_csvs) < 2:
        return

    entries = []
    for csv_path in ext_stats_csvs:
        name = Path(csv_path).stem
        soi_type = name.replace('ext_stats_', '')
        df = pd.read_csv(csv_path)
        wide = _pivot_stats(df, 'soi_workers')
        warmup_s, duration_s = _parse_params(csv_path)
        wide = _add_elapsed(wide, 'soi_workers', max_elapsed_s=warmup_s + duration_s)
        entries.append((soi_type, wide))
    entries.sort(key=lambda e: e[0])

    out_dir = str(Path(ext_stats_csvs[0]).parent.parent)
    print(f"  RocksDB SoI comparison -> {out_dir}/")

    has_stall = all('cumulative_stall_pct' in w.columns for _, w in entries)
    has_wa = all('write_amp' in w.columns for _, w in entries)

    if not has_stall and not has_wa:
        return

    rows = []
    for soi_type, wide in entries:
        groups = sorted(wide['soi_workers'].unique())
        for g in groups:
            sub = wide[wide['soi_workers'] == g]
            finals = sub.groupby('sample_idx').last()
            row = {'soi_type': soi_type, 'soi_workers': g}
            if has_stall:
                row['stall'] = finals['cumulative_stall_pct'].median()
            if has_wa:
                row['write_amp'] = finals['write_amp'].median()
            rows.append(row)
    cdf = pd.DataFrame(rows)

    max_soi = int(cdf['soi_workers'].max())

    if has_stall:
        final = cdf[cdf['soi_workers'] == max_soi].copy()
        final = final.sort_values('stall', ascending=False)
        fig, ax = plt.subplots(figsize=(10, 5))
        colors = [SOI_COLORS.get(t, 'gray') for t in final['soi_type']]
        bars = ax.bar(final['soi_type'], final['stall'], color=colors, alpha=0.8,
                      edgecolor='black', linewidth=0.5)
        for bar, val in zip(bars, final['stall']):
            ax.text(bar.get_x() + bar.get_width() / 2, val + 0.3,
                    f'{val:.1f}%', ha='center', va='bottom', fontsize=9, fontweight='bold')
        ax.set_xlabel('SoI Type')
        ax.set_ylabel('Cumulative Write Stall (%)')
        ax.set_title(f'RocksDB Write Stall at {max_soi} SoI Workers')
        ax.set_ylim(0, max(final['stall'].max() * 1.3, 10))
        ax.grid(True, alpha=0.3, axis='y')
        save_fig(fig, os.path.join(out_dir, 'soi_comparison_stall.png'))

    if has_wa:
        final = cdf[cdf['soi_workers'] == max_soi].copy()
        final = final.sort_values('write_amp', ascending=False)
        fig, ax = plt.subplots(figsize=(10, 5))
        colors = [SOI_COLORS.get(t, 'gray') for t in final['soi_type']]
        bars = ax.bar(final['soi_type'], final['write_amp'], color=colors, alpha=0.8,
                      edgecolor='black', linewidth=0.5)
        for bar, val in zip(bars, final['write_amp']):
            ax.text(bar.get_x() + bar.get_width() / 2, val + 0.03,
                    f'{val:.1f}x', ha='center', va='bottom', fontsize=9, fontweight='bold')
        ax.set_xlabel('SoI Type')
        ax.set_ylabel('Write Amplification')
        ax.set_title(f'RocksDB Write Amplification at {max_soi} SoI Workers')
        ax.set_ylim(0, max(final['write_amp'].max() * 1.3, 2))
        ax.grid(True, alpha=0.3, axis='y')
        save_fig(fig, os.path.join(out_dir, 'soi_comparison_write_amp.png'))

    if has_stall:
        fig, ax = plt.subplots(figsize=(10, 6))
        for soi_type, _ in entries:
            sub = cdf[cdf['soi_type'] == soi_type]
            color = SOI_COLORS.get(soi_type, 'gray')
            marker = SOI_MARKERS.get(soi_type, 'o')
            ax.plot(sub['soi_workers'], sub['stall'], f'-{marker}', color=color,
                    linewidth=2, markersize=6, label=soi_type)
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('Cumulative Write Stall (%)')
        ax.set_title('RocksDB Write Stall vs SoI Workers')
        ax.set_xlim(left=0)
        ax.set_ylim(bottom=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend(loc='best', ncol=2, fontsize=9)
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(out_dir, 'soi_comparison_stall_curve.png'))


def process_single(csv_path):
    """Process a single ext_stats CSV file."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)

    is_saturation = 'saturation' in name
    if is_saturation:
        group_col = 'concurrency'
        group_label = 'Concurrency (N)'
        title_prefix = 'External Saturation'
    else:
        group_col = 'soi_workers'
        group_label = 'SoI Workers'
        soi_type = name.replace('ext_stats_', '')
        title_prefix = f'SoI Sweep ({soi_type})'

    if group_col not in df.columns:
        print(f"  Skipping {name}: missing column '{group_col}'")
        return

    print(f"  {csv_path}")
    wide = _pivot_stats(df, group_col)
    warmup_s, duration_s = _parse_params(csv_path)
    wide = _add_elapsed(wide, group_col, max_elapsed_s=warmup_s + duration_s)

    ts_df = _load_throughput_timeseries(csv_path, group_col)

    plot_stall_timeseries(wide, group_col, group_label, folder, title_prefix, warmup_s)
    plot_stall_heatmap(wide, group_col, group_label, folder, title_prefix, warmup_s)
    plot_write_amp_vs_group(wide, group_col, group_label, folder, title_prefix)
    plot_compaction_rate(wide, group_col, group_label, folder, title_prefix, warmup_s)
    plot_compaction_write_rate(wide, group_col, group_label, folder, title_prefix, warmup_s)
    plot_stall_vs_throughput(wide, ts_df, group_col, group_label, folder, title_prefix, warmup_s)
    plot_aggregate_summary(wide, group_col, group_label, folder, title_prefix)
    plot_stall_throughput_dual(wide, ts_df, group_col, group_label, folder, title_prefix, warmup_s)


def find_ext_stats(paths):
    """Find all ext_stats CSV files from given paths."""
    csv_files = []
    for p in paths:
        if os.path.isfile(p) and 'ext_stats' in Path(p).stem:
            csv_files.append(p)
        elif os.path.isdir(p):
            csv_files.extend(
                glob.glob(os.path.join(p, '**', 'ext_stats*.csv'), recursive=True)
            )
    return sorted(csv_files)


def find_soi_groups(csv_files):
    """Group SoI ext_stats files that share a parent workload directory
    (e.g. all SoI types under fill_random/) for cross-SoI comparison plots."""
    groups = {}
    for f in csv_files:
        name = Path(f).stem
        if 'saturation' in name:
            continue
        workload_dir = str(Path(f).parent.parent)
        groups.setdefault(workload_dir, []).append(f)
    return groups


def main():
    paths = sys.argv[1:] or ['./completed_output']
    csv_files = find_ext_stats(paths)
    if not csv_files:
        print("No ext_stats CSV files found.")
        return

    print(f"Found {len(csv_files)} ext_stats CSV(s)")
    for f in csv_files:
        process_single(f)

    for workload_dir, group_csvs in find_soi_groups(csv_files).items():
        if len(group_csvs) >= 2:
            plot_soi_comparison_rocksdb(group_csvs)


if __name__ == '__main__':
    main()
