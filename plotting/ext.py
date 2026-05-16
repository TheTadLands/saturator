"""Plots for external-workload (victim) experiments — SoI sweeps and saturation."""

import os
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
from matplotlib.ticker import MaxNLocator

from .common import (
    C_BLUE, C_CYAN, C_GREEN, C_ORANGE, C_PURPLE, C_RED,
    ext_subdir, format_number, plot_perf_metrics, save_fig,
)


def _plot_combined_per_group(df, groups, group_col, folder, title_prefix, group_label, out_template):
    """For each value in `groups`, render one figure with throughput on the left
    axis and CPU/IO/IOPS/memory utilizations overlaid on a shared right axis.
    Lets you see within-window phase correlations that stacked small-multiples hide.
    """
    has_cpu = 'cpu_pct' in df.columns
    has_io_bw = 'io_util_pct' in df.columns
    has_io_iops = 'io_iops_util_pct' in df.columns
    has_mem = 'mem_usage_pct' in df.columns

    for g in groups:
        sub = df[df[group_col] == g].sort_values('elapsed_ms')
        if sub.empty:
            continue

        fig, ax_tp = plt.subplots(figsize=(11, 5))
        t = sub['elapsed_ms'] / 1000

        line_tp, = ax_tp.plot(t, sub['ext_ops_sec'], '-', color=C_BLUE,
                              linewidth=2.2, label='Throughput')
        ax_tp.set_xlabel('Elapsed (seconds)')
        ax_tp.set_ylabel('External Throughput (ops/sec)', color=C_BLUE)
        ax_tp.tick_params(axis='y', labelcolor=C_BLUE)
        ax_tp.set_ylim(bottom=0)
        ax_tp.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        ax_tp.grid(True, alpha=0.3)

        ax_util = ax_tp.twinx()
        lines = [line_tp]
        if has_cpu:
            l, = ax_util.plot(t, sub['cpu_pct'], '-', color=C_ORANGE,
                              linewidth=1.2, alpha=0.8, label='CPU%')
            lines.append(l)
        if has_io_bw:
            l, = ax_util.plot(t, sub['io_util_pct'], '-', color=C_GREEN,
                              linewidth=1.2, alpha=0.8, label='IO BW%')
            lines.append(l)
        if has_io_iops:
            l, = ax_util.plot(t, sub['io_iops_util_pct'], '-', color=C_PURPLE,
                              linewidth=1.2, alpha=0.8, label='IO IOPS%')
            lines.append(l)
        if has_mem:
            l, = ax_util.plot(t, sub['mem_usage_pct'], '-', color=C_RED,
                              linewidth=1.2, alpha=0.8, label='Mem%')
            lines.append(l)
        ax_util.set_ylabel('Utilization (%)')
        ax_util.set_ylim(0, 105)

        ax_tp.set_title(f'{title_prefix} — {group_label}={int(g)}')
        ax_tp.legend(lines, [l.get_label() for l in lines], loc='upper right', fontsize=9)
        fig.tight_layout()
        save_fig(fig, os.path.join(ext_subdir(folder, 'timeseries'), out_template.format(int(g))))


def plot_per_sample_ext_soi(csv_path: str):
    """Scatter + per-sample curves from the long-format per-sample SoI sweep CSV."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)
    name = Path(csv_path).stem
    soi_type = name.replace('per_sample_ext_soi_', '')

    base = df[df['soi_workers'] == 0]['ext_ops_sec']
    base_mean = base.mean() if len(base) else 0

    fig, ax = plt.subplots(figsize=(10, 5))
    ax.scatter(df['soi_workers'], df['ext_ops_sec'], s=40, alpha=0.55,
               color=C_BLUE, edgecolors='white', linewidths=0.5, label='Sample')
    means = df.groupby('soi_workers')['ext_ops_sec'].mean().reset_index()
    ax.plot(means['soi_workers'], means['ext_ops_sec'], '-o', color=C_RED,
            linewidth=2, markersize=6, label='Mean')
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('External Throughput (ops/sec)')
    ax.set_title(f'SoI Sweep ({soi_type}) — Per-Sample Throughput')
    ax.set_ylim(bottom=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    ax.legend(loc='best')
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(ext_subdir(folder, 'per_sample'), 'scatter.png'))

    fig, ax = plt.subplots(figsize=(10, 5))
    cmap = plt.cm.viridis
    sample_ids = sorted(df['sample_idx'].unique())
    for i, sid in enumerate(sample_ids):
        sub = df[df['sample_idx'] == sid].sort_values('soi_workers')
        color = cmap(i / max(len(sample_ids) - 1, 1))
        ax.plot(sub['soi_workers'], sub['ext_ops_sec'], '-o', color=color,
                linewidth=1.5, markersize=5, alpha=0.85, label=f'Sample {int(sid)}')
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('External Throughput (ops/sec)')
    ax.set_title(f'SoI Sweep ({soi_type}) — Per-Sample Curves')
    ax.set_ylim(bottom=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    ax.legend(loc='best', fontsize=9)
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(ext_subdir(folder, 'per_sample'), 'curves.png'))

    if base_mean > 0:
        fig, ax = plt.subplots(figsize=(10, 5))
        df_deg = df.copy()
        df_deg['deg_pct'] = (df_deg['ext_ops_sec'] - base_mean) / base_mean * 100
        ax.scatter(df_deg['soi_workers'], df_deg['deg_pct'], s=40, alpha=0.55,
                   color=C_BLUE, edgecolors='white', linewidths=0.5, label='Sample')
        means = df_deg.groupby('soi_workers')['deg_pct'].mean().reset_index()
        ax.plot(means['soi_workers'], means['deg_pct'], '-o', color=C_RED,
                linewidth=2, markersize=6, label='Mean')
        ax.axhline(0, color='black', linewidth=0.8, alpha=0.4)
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('Throughput Change (%)')
        ax.set_title(f'SoI Sweep ({soi_type}) — Per-Sample Degradation')
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend(loc='best')
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(ext_subdir(folder, 'per_sample'), 'degradation.png'))


def plot_per_sample_ext_saturation(csv_path: str):
    """Scatter + per-sample curves for external saturation per-sample CSV."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    fig, ax = plt.subplots(figsize=(10, 5))
    ax.scatter(df['concurrency'], df['ext_ops_sec'], s=40, alpha=0.55,
               color=C_BLUE, edgecolors='white', linewidths=0.5, label='Sample')
    means = df.groupby('concurrency')['ext_ops_sec'].mean().reset_index()
    ax.plot(means['concurrency'], means['ext_ops_sec'], '-o', color=C_RED,
            linewidth=2, markersize=6, label='Mean')
    ax.set_xlabel('Concurrency (N)')
    ax.set_ylabel('External Throughput (ops/sec)')
    ax.set_title('External Workload Saturation — Per-Sample Throughput')
    ax.set_ylim(bottom=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    ax.legend(loc='best')
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(ext_subdir(folder, 'per_sample'), 'scatter.png'))


def plot_ext_timeseries(csv_path: str):
    """Generate time-series plots for external workload SoI sweep."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)

    soi_type = name.replace('timeseries_ext_soi_', '')

    soi_counts = sorted(df['soi_workers'].unique())

    print(f"  -> {folder}/")

    fig, ax = plt.subplots(figsize=(10, 6))
    cmap = plt.cm.viridis
    for i, s in enumerate(soi_counts):
        sub = df[df['soi_workers'] == s]
        color = cmap(i / max(len(soi_counts) - 1, 1))
        ax.plot(sub['elapsed_ms'] / 1000, sub['ext_ops_sec'], '-', color=color,
                linewidth=1.2, alpha=0.8, label=f'{int(s)} SoI')
    ax.set_xlabel('Elapsed (seconds)')
    ax.set_ylabel('External Throughput (ops/sec)')
    ax.set_title(f'SoI Sweep ({soi_type}) — External Throughput Time Series')
    ax.set_ylim(bottom=0)
    ax.legend(loc='best', fontsize=8, ncol=2)
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(ext_subdir(folder, 'timeseries'), 'ext_throughput.png'))

    if len(soi_counts) > 1:
        pivot = df.pivot_table(index='soi_workers', columns='elapsed_ms', values='ext_ops_sec')
        fig, ax = plt.subplots(figsize=(12, 5))
        im = ax.imshow(pivot.values, aspect='auto', cmap='viridis',
                       extent=[pivot.columns.min() / 1000, pivot.columns.max() / 1000,
                               pivot.index.max() + 0.5, pivot.index.min() - 0.5])
        ax.set_xlabel('Elapsed (seconds)')
        ax.set_ylabel('SoI Workers')
        ax.set_title(f'SoI Sweep ({soi_type}) — Throughput Heatmap')
        ax.set_yticks(soi_counts)
        cbar = fig.colorbar(im, ax=ax, label='External ops/sec')
        cbar.formatter = plt.FuncFormatter(lambda v, p: format_number(v))
        cbar.update_ticks()
        save_fig(fig, os.path.join(ext_subdir(folder, 'timeseries'), 'heatmap.png'))

    _plot_combined_per_group(
        df, soi_counts, 'soi_workers', folder,
        title_prefix=f'SoI Sweep ({soi_type})',
        group_label='SoI',
        out_template='timeseries_per_soi_{:02d}.png',
    )

    has_soi_cpu_ts = 'soi_cpu_ops_sec' in df.columns and df['soi_cpu_ops_sec'].max() > 0
    has_soi_io_ts = 'soi_io_ops_sec' in df.columns and df['soi_io_ops_sec'].max() > 0
    has_soi_both_ts = has_soi_cpu_ts and has_soi_io_ts
    if 'soi_ops_sec' in df.columns:
        soi_df = df[df['soi_workers'] > 0]
        if soi_df['soi_ops_sec'].max() > 0:
            soi_nonzero = sorted(soi_df['soi_workers'].unique())

            if has_soi_both_ts:
                fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 8), sharex=True,
                                                     layout='constrained')
                cmap = plt.cm.plasma
                for i, s in enumerate(soi_nonzero):
                    sub = soi_df[soi_df['soi_workers'] == s]
                    color = cmap(i / max(len(soi_nonzero) - 1, 1))
                    ax_cpu.plot(sub['elapsed_ms'] / 1000, sub['soi_cpu_ops_sec'], '-', color=color,
                                linewidth=1.2, alpha=0.8, label=f'{int(s)} SoI')
                    ax_io.plot(sub['elapsed_ms'] / 1000, sub['soi_io_ops_sec'], '-', color=color,
                               linewidth=1.2, alpha=0.8, label=f'{int(s)} SoI')
                ax_cpu.set_ylabel('SoI CPU ops/sec')
                ax_cpu.set_ylim(bottom=0)
                ax_cpu.legend(loc='best', fontsize=8, ncol=2)
                ax_cpu.grid(True, alpha=0.3)
                ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
                ax_io.set_xlabel('Elapsed (seconds)')
                ax_io.set_ylabel('SoI IO ops/sec')
                ax_io.set_ylim(bottom=0)
                ax_io.legend(loc='best', fontsize=8, ncol=2)
                ax_io.grid(True, alpha=0.3)
                ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
                fig.suptitle(f'SoI Sweep ({soi_type}) — SoI Throughput Time Series')
            else:
                fig, ax = plt.subplots(figsize=(10, 6))
                cmap = plt.cm.plasma
                for i, s in enumerate(soi_nonzero):
                    sub = soi_df[soi_df['soi_workers'] == s]
                    color = cmap(i / max(len(soi_nonzero) - 1, 1))
                    ax.plot(sub['elapsed_ms'] / 1000, sub['soi_ops_sec'], '-', color=color,
                            linewidth=1.2, alpha=0.8, label=f'{int(s)} SoI')
                ax.set_xlabel('Elapsed (seconds)')
                ax.set_ylabel('SoI Throughput (ops/sec)')
                ax.set_title(f'SoI Sweep ({soi_type}) — SoI Throughput Time Series')
                ax.set_ylim(bottom=0)
                ax.legend(loc='best', fontsize=8, ncol=2)
                ax.grid(True, alpha=0.3)
                ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
            save_fig(fig, os.path.join(ext_subdir(folder, 'timeseries'), 'soi_throughput.png'))

            if len(soi_nonzero) > 1:
                pivot = soi_df.pivot_table(index='soi_workers', columns='elapsed_ms', values='soi_ops_sec')
                fig, ax = plt.subplots(figsize=(12, 5))
                im = ax.imshow(pivot.values, aspect='auto', cmap='plasma',
                               extent=[pivot.columns.min() / 1000, pivot.columns.max() / 1000,
                                       pivot.index.max() + 0.5, pivot.index.min() - 0.5])
                ax.set_xlabel('Elapsed (seconds)')
                ax.set_ylabel('SoI Workers')
                ax.set_title(f'SoI Sweep ({soi_type}) — SoI Throughput Heatmap')
                ax.set_yticks(soi_nonzero)
                cbar = fig.colorbar(im, ax=ax, label='SoI ops/sec')
                cbar.formatter = plt.FuncFormatter(lambda v, p: format_number(v))
                cbar.update_ticks()
                save_fig(fig, os.path.join(ext_subdir(folder, 'timeseries'), 'soi_heatmap.png'))

    if 'cpu_pct' in df.columns and 'io_util_pct' in df.columns:
        stats = []
        for s in soi_counts:
            sub = df[df['soi_workers'] == s]
            stats.append({
                'soi': s,
                'cpu_peak': sub['cpu_pct'].max(),
                'cpu_p95': sub['cpu_pct'].quantile(0.95),
                'cpu_median': sub['cpu_pct'].median(),
                'io_peak': sub['io_util_pct'].max(),
                'io_p95': sub['io_util_pct'].quantile(0.95),
                'io_median': sub['io_util_pct'].median(),
            })
        sdf = pd.DataFrame(stats)

        fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 7), sharex=True,
                                             layout='constrained')
        ax_cpu.fill_between(sdf['soi'], sdf['cpu_median'], sdf['cpu_peak'],
                            alpha=0.15, color=C_ORANGE)
        ax_cpu.fill_between(sdf['soi'], sdf['cpu_median'], sdf['cpu_p95'],
                            alpha=0.25, color=C_ORANGE)
        ax_cpu.plot(sdf['soi'], sdf['cpu_peak'], '-^', color=C_RED, linewidth=2,
                    markersize=6, label='Peak')
        ax_cpu.plot(sdf['soi'], sdf['cpu_p95'], '-s', color=C_ORANGE, linewidth=2,
                    markersize=5, label='P95')
        ax_cpu.plot(sdf['soi'], sdf['cpu_median'], '-o', color=C_BLUE, linewidth=2,
                    markersize=5, label='Median')
        ax_cpu.set_ylabel('CPU Utilization (%)')
        ax_cpu.set_ylim(0, 105)
        ax_cpu.legend(loc='best', fontsize=9)
        ax_cpu.grid(True, alpha=0.3)
        ax_cpu.set_title(f'SoI Sweep ({soi_type}) — Utilization Distribution (Peak / P95 / Median)')

        ax_io.fill_between(sdf['soi'], sdf['io_median'], sdf['io_peak'],
                           alpha=0.15, color=C_GREEN)
        ax_io.fill_between(sdf['soi'], sdf['io_median'], sdf['io_p95'],
                           alpha=0.25, color=C_GREEN)
        ax_io.plot(sdf['soi'], sdf['io_peak'], '-^', color=C_RED, linewidth=2,
                   markersize=6, label='Peak')
        ax_io.plot(sdf['soi'], sdf['io_p95'], '-s', color=C_GREEN, linewidth=2,
                   markersize=5, label='P95')
        ax_io.plot(sdf['soi'], sdf['io_median'], '-o', color=C_BLUE, linewidth=2,
                   markersize=5, label='Median')
        ax_io.set_xlabel('SoI Workers')
        ax_io.set_ylabel('IO BW Utilization (%)')
        ax_io.set_ylim(0, 105)
        ax_io.legend(loc='best', fontsize=9)
        ax_io.grid(True, alpha=0.3)
        ax_io.xaxis.set_major_locator(MaxNLocator(integer=True))
        save_fig(fig, os.path.join(ext_subdir(folder, 'utilization'), 'distribution.png'))

    if 'cpu_pct' in df.columns and 'io_util_pct' in df.columns:
        baseline = df[df['soi_workers'] == soi_counts[0]]
        if len(baseline) > 5:
            fig, ax = plt.subplots(figsize=(8, 5))
            for col, color, label in [('cpu_pct', C_ORANGE, 'CPU'),
                                       ('io_util_pct', C_GREEN, 'IO BW')]:
                sorted_vals = np.sort(baseline[col].values)
                cdf = np.arange(1, len(sorted_vals) + 1) / len(sorted_vals)
                ax.plot(sorted_vals, cdf * 100, '-', color=color, linewidth=2, label=label)
            ax.set_xlabel('Utilization (%)')
            ax.set_ylabel('Percentile')
            ax.set_title(f'SoI Sweep ({soi_type}) — Baseline Utilization CDF (0 SoI workers)')
            ax.set_xlim(0, 105)
            ax.set_ylim(0, 100)
            ax.legend(loc='best', fontsize=9)
            ax.grid(True, alpha=0.3)
            save_fig(fig, os.path.join(ext_subdir(folder, 'utilization'), 'cdf_baseline.png'))


def plot_ext_saturation_timeseries(csv_path: str):
    """Generate time-series plots for external workload saturation experiments."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    concurrencies = sorted(df['concurrency'].unique())

    print(f"  -> {folder}/")

    fig, ax = plt.subplots(figsize=(10, 6))
    cmap = plt.cm.viridis
    for i, n in enumerate(concurrencies):
        sub = df[df['concurrency'] == n]
        color = cmap(i / max(len(concurrencies) - 1, 1))
        ax.plot(sub['elapsed_ms'] / 1000, sub['ext_ops_sec'], '-', color=color,
                linewidth=1.2, alpha=0.8, label=f'N={int(n)}')
    ax.set_xlabel('Elapsed (seconds)')
    ax.set_ylabel('External Throughput (ops/sec)')
    ax.set_title('External Workload Saturation — Throughput Time Series')
    ax.set_ylim(bottom=0)
    ax.legend(loc='best', fontsize=8, ncol=2)
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(ext_subdir(folder, 'timeseries'), 'ext_throughput.png'))

    if len(concurrencies) > 1:
        pivot = df.pivot_table(index='concurrency', columns='elapsed_ms', values='ext_ops_sec')
        fig, ax = plt.subplots(figsize=(12, 5))
        im = ax.imshow(pivot.values, aspect='auto', cmap='viridis',
                       extent=[pivot.columns.min() / 1000, pivot.columns.max() / 1000,
                               pivot.index.max() + 0.5, pivot.index.min() - 0.5])
        ax.set_xlabel('Elapsed (seconds)')
        ax.set_ylabel('Concurrency (N)')
        ax.set_title('External Workload Saturation — Throughput Heatmap')
        ax.set_yticks(concurrencies)
        cbar = fig.colorbar(im, ax=ax, label='External ops/sec')
        cbar.formatter = plt.FuncFormatter(lambda v, p: format_number(v))
        cbar.update_ticks()
        save_fig(fig, os.path.join(ext_subdir(folder, 'timeseries'), 'heatmap.png'))

    _plot_combined_per_group(
        df, concurrencies, 'concurrency', folder,
        title_prefix='External Workload Saturation',
        group_label='N',
        out_template='timeseries_per_n_{:02d}.png',
    )


def plot_ext_saturation(csv_path: str):
    """Generate graphs for external workload saturation CSV."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)
    x = df['concurrency']

    has_stddev = 'ext_ops_stddev' in df.columns
    has_pcts = {'ops_sec_p10', 'ops_sec_p50', 'ops_sec_p90'}.issubset(df.columns)

    print(f"  -> {folder}/")

    peak_idx = df['ext_ops_sec'].idxmax()
    peak_n = df.loc[peak_idx, 'concurrency']
    peak_ops = df.loc[peak_idx, 'ext_ops_sec']

    fig, ax = plt.subplots(figsize=(8, 5))
    if has_pcts:
        ax.fill_between(x, df['ops_sec_p10'], df['ops_sec_p90'],
                        alpha=0.2, color=C_BLUE, label='p10–p90')
        ax.plot(x, df['ops_sec_p50'], '--', color=C_BLUE, alpha=0.6, label='p50')
        ax.plot(x, df['ext_ops_sec'], '-o', color=C_BLUE, linewidth=2,
                markersize=6, label='Mean ops/s')
    else:
        ax.errorbar(x, df['ext_ops_sec'],
                    yerr=df['ext_ops_stddev'] if has_stddev else None,
                    fmt='-o', color=C_BLUE, linewidth=2, markersize=6,
                    capsize=3, capthick=1, label='External ops/s')
    ax.axvline(x=peak_n, color=C_RED, linestyle='--', alpha=0.7,
               label=f'Peak: N={int(peak_n)}')
    ax.scatter([peak_n], [peak_ops], color=C_RED, s=100, zorder=5)
    ax.set_xlabel('Concurrency (N)')
    ax.set_ylabel('Throughput (ops/sec)')
    ax.set_title('External Workload Saturation')
    ax.set_ylim(bottom=0)
    ax.set_xlim(left=1)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.legend()
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'throughput_total.png'))

    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(x, df['throughput_per_unit'], '-s', color=C_BLUE, linewidth=2, markersize=6)
    ax.set_xlabel('Concurrency (N)')
    ax.set_ylabel('Throughput per Unit (ops/sec)')
    ax.set_title('External Workload — Per-Unit Efficiency')
    ax.set_ylim(bottom=0)
    ax.set_xlim(left=1)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'throughput_per_unit.png'))

    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO BW %')
        if 'io_iops_util_pct' in df.columns:
            ax.plot(x, df['io_iops_util_pct'], '-^', color=C_CYAN, linewidth=2, markersize=6, label='IO IOPS %')
        if 'mem_usage_pct' in df.columns:
            ax.plot(x, df['mem_usage_pct'], '-D', color=C_PURPLE, linewidth=2, markersize=6, label='Memory %')
        ax.set_xlabel('Concurrency (N)')
        ax.set_ylabel('Utilization (%)')
        ax.set_title('External Workload Saturation — Resource Utilization')
        ax.set_ylim(0, 105)
        ax.set_xlim(left=1)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(ext_subdir(folder, 'utilization'), 'utilization.png'))

    # Hardware perf counters (cache misses, IPC)
    plot_perf_metrics(df, 'concurrency', 'Concurrency (N)',
                      'External Saturation', folder,
                      throughput_col='ext_ops_sec')


def plot_ext_soi(csv_path: str):
    """Generate graphs for external workload SoI sweep CSV."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)
    x = df['soi_workers']

    soi_type = name.replace('ext_soi_', '').replace('_throughput', '')

    has_stddev = 'ext_ops_stddev' in df.columns
    has_pcts = {'ops_sec_p10', 'ops_sec_p50', 'ops_sec_p90'}.issubset(df.columns)

    print(f"  -> {folder}/")

    fig, ax = plt.subplots(figsize=(8, 5))
    if has_pcts:
        ax.fill_between(x, df['ops_sec_p10'], df['ops_sec_p90'],
                        alpha=0.2, color=C_BLUE, label='p10–p90')
        ax.plot(x, df['ops_sec_p50'], '--', color=C_BLUE, alpha=0.6, label='p50')
        ax.plot(x, df['ext_ops_sec'], '-o', color=C_BLUE, linewidth=2,
                markersize=6, label='Mean ops/s')
    else:
        ax.errorbar(x, df['ext_ops_sec'],
                    yerr=df['ext_ops_stddev'] if has_stddev else None,
                    fmt='-o', color=C_BLUE, linewidth=2, markersize=6,
                    capsize=3, capthick=1, label='External ops/s')
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('External Throughput (ops/sec)')
    ax.set_title(f'SoI Sweep ({soi_type}) — External Workload Throughput')
    ax.set_ylim(bottom=0)
    ax.set_xlim(left=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.legend(loc='best', fontsize=9)
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'victim_throughput.png'))

    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(x, df['ext_change_pct'], '-o', color=C_RED, linewidth=2, markersize=5)
    ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    ax.fill_between(x, df['ext_change_pct'], 0,
                    where=(df['ext_change_pct'] < 0), interpolate=True, alpha=0.2, color=C_RED)
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('Throughput Change (%)')
    ax.set_title(f'SoI Sweep ({soi_type}) — External Workload Degradation')
    ax.set_xlim(left=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'victim_degradation.png'))

    has_soi_cpu_agg = 'soi_cpu_ops' in df.columns and df['soi_cpu_ops'].max() > 0
    has_soi_io_agg = 'soi_io_ops' in df.columns and df['soi_io_ops'].max() > 0
    if df['soi_ops'].max() > 0:
        mask = x > 0
        if has_soi_cpu_agg and has_soi_io_agg:
            fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(8, 7), sharex=True,
                                                 layout='constrained')
            ax_cpu.plot(x[mask], df.loc[mask, 'soi_cpu_ops'], '-^', color=C_BLUE,
                        linewidth=2, markersize=6, label='SoI CPU ops/s')
            ax_cpu.set_ylabel('SoI CPU ops/sec')
            ax_cpu.set_ylim(bottom=0)
            ax_cpu.legend(loc='best', fontsize=9)
            ax_cpu.grid(True, alpha=0.3)
            ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
            ax_cpu.set_title(f'SoI Sweep ({soi_type}) — SoI Throughput')

            ax_io.plot(x[mask], df.loc[mask, 'soi_io_ops'], '-s', color=C_GREEN,
                       linewidth=2, markersize=6, label='SoI IO ops/s')
            ax_io.set_xlabel('SoI Workers')
            ax_io.set_ylabel('SoI IO ops/sec')
            ax_io.set_ylim(bottom=0)
            ax_io.xaxis.set_major_locator(MaxNLocator(integer=True))
            ax_io.legend(loc='best', fontsize=9)
            ax_io.grid(True, alpha=0.3)
            ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        else:
            fig, ax = plt.subplots(figsize=(8, 5))
            if has_soi_cpu_agg:
                ax.plot(x[mask], df.loc[mask, 'soi_cpu_ops'], '-^', color=C_BLUE,
                        linewidth=2, markersize=6, label='SoI CPU ops/s')
            elif has_soi_io_agg:
                ax.plot(x[mask], df.loc[mask, 'soi_io_ops'], '-s', color=C_GREEN,
                        linewidth=2, markersize=6, label='SoI IO ops/s')
            else:
                ax.plot(x[mask], df.loc[mask, 'soi_ops'], '-^', color=C_ORANGE,
                        linewidth=2, markersize=6, label='SoI ops/s')
            ax.set_xlabel('SoI Workers')
            ax.set_ylabel('SoI Throughput (ops/sec)')
            ax.set_title(f'SoI Sweep ({soi_type}) — SoI Throughput')
            ax.set_ylim(bottom=0)
            ax.set_xlim(left=0)
            ax.xaxis.set_major_locator(MaxNLocator(integer=True))
            ax.legend(loc='best', fontsize=9)
            ax.grid(True, alpha=0.3)
            ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        save_fig(fig, os.path.join(folder, 'soi_throughput.png'))

    fig, ax1 = plt.subplots(figsize=(8, 5))
    line_tp = ax1.plot(x, df['ext_ops_sec'], '-o', color=C_BLUE, linewidth=2,
                       markersize=6, label='External ops/s')
    ax1.set_xlabel('SoI Workers')
    ax1.set_ylabel('External Throughput (ops/sec)', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.set_ylim(bottom=0)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax2 = ax1.twinx()
    line_deg = ax2.plot(x, df['ext_change_pct'], '-s', color=C_RED, linewidth=2,
                        markersize=5, label='Change %')
    ax2.set_ylabel('Throughput Change (%)', color=C_RED)
    ax2.tick_params(axis='y', labelcolor=C_RED)

    lines = line_tp + line_deg
    ax1.legend(lines, [l.get_label() for l in lines], loc='best')
    ax1.set_title(f'SoI Sweep ({soi_type}) — Throughput vs Degradation')
    ax1.set_xlim(left=0)
    ax1.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'throughput_vs_degradation.png'))

    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO BW %')
        if 'io_iops_util_pct' in df.columns:
            ax.plot(x, df['io_iops_util_pct'], '-^', color=C_CYAN, linewidth=2, markersize=6, label='IO IOPS %')
        if 'mem_usage_pct' in df.columns:
            ax.plot(x, df['mem_usage_pct'], '-D', color=C_PURPLE, linewidth=2, markersize=6, label='Memory %')
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('Utilization (%)')
        ax.set_title(f'SoI Sweep ({soi_type}) — Resource Utilization')
        ax.set_ylim(0, 105)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(ext_subdir(folder, 'utilization'), 'utilization.png'))

    # Hardware perf counters (cache misses, IPC)
    plot_perf_metrics(df, 'soi_workers', 'SoI Workers',
                      f'SoI Sweep ({soi_type})', folder,
                      throughput_col='ext_ops_sec')


SOI_COLORS = {
    'cpu': C_BLUE, 'l1d': C_CYAN, 'l2': C_PURPLE, 'l3': C_GREEN,
    'membw': C_ORANGE, 'memcap': '#999999', 'iobw': C_RED, 'iops': '#8B4513',
}
SOI_MARKERS = {
    'cpu': 'o', 'l1d': 's', 'l2': '^', 'l3': 'D',
    'membw': 'v', 'memcap': 'x', 'iobw': 'P', 'iops': '*',
}


def plot_ext_soi_comparison(ext_soi_csvs: list):
    """Generate comparison plots across multiple external SoI types."""
    if len(ext_soi_csvs) < 2:
        return

    entries = []
    for csv_path in ext_soi_csvs:
        df = pd.read_csv(csv_path)
        name = Path(csv_path).stem
        soi_type = name.replace('ext_soi_', '').replace('_throughput', '')
        entries.append((soi_type, df, csv_path))
    entries.sort(key=lambda e: e[0])

    out_dir = str(Path(ext_soi_csvs[0]).parent.parent)
    print(f"  Ext SoI comparison -> {out_dir}/")

    fig, ax = plt.subplots(figsize=(10, 6))
    for soi_type, df, _ in entries:
        color = SOI_COLORS.get(soi_type, 'gray')
        marker = SOI_MARKERS.get(soi_type, 'o')
        ax.plot(df['soi_workers'], df['ext_change_pct'], f'-{marker}',
                color=color, linewidth=2, markersize=6, label=soi_type)
    ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('External Throughput Change (%)')
    ax.set_title('SoI Comparison — External Workload Degradation')
    ax.set_xlim(left=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.legend(loc='best', ncol=2, fontsize=9)
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_degradation.png'))

    fig, ax = plt.subplots(figsize=(10, 5))
    types = []
    final_degs = []
    colors = []
    for soi_type, df, _ in entries:
        types.append(soi_type)
        final_degs.append(df['ext_change_pct'].iloc[-1])
        colors.append(SOI_COLORS.get(soi_type, 'gray'))
    order = sorted(range(len(final_degs)), key=lambda i: final_degs[i])
    types = [types[i] for i in order]
    final_degs = [final_degs[i] for i in order]
    colors = [colors[i] for i in order]
    bars = ax.bar(types, final_degs, color=colors, alpha=0.8, edgecolor='black', linewidth=0.5)
    ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    for bar, val in zip(bars, final_degs):
        y_pos = val - 1.5 if val < 0 else val + 0.5
        va = 'top' if val < 0 else 'bottom'
        ax.text(bar.get_x() + bar.get_width() / 2, y_pos,
                f'{val:.1f}%', ha='center', va=va, fontsize=9, fontweight='bold')
    ax.set_xlabel('SoI Type')
    ax.set_ylabel('External Throughput Change (%)')
    max_soi = int(entries[0][1]['soi_workers'].max())
    ax.set_title(f'SoI Impact at {max_soi} Workers — External Workload')
    ax.grid(True, alpha=0.3, axis='y')
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_bar.png'))

    fig, ax = plt.subplots(figsize=(10, 6))
    for soi_type, df, _ in entries:
        color = SOI_COLORS.get(soi_type, 'gray')
        marker = SOI_MARKERS.get(soi_type, 'o')
        baseline = df['ext_ops_sec'].iloc[0]
        if baseline > 0:
            normalized = df['ext_ops_sec'] / baseline * 100
            ax.plot(df['soi_workers'], normalized, f'-{marker}',
                    color=color, linewidth=2, markersize=6, label=soi_type)
    ax.axhline(y=100, color='gray', linestyle='--', alpha=0.5, label='Baseline')
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('External Throughput (% of baseline)')
    ax.set_title('SoI Comparison — Normalized External Throughput')
    ax.set_xlim(left=0)
    ax.set_ylim(bottom=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.legend(loc='best', ncol=2, fontsize=9)
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_normalized.png'))
