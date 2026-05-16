"""Plots for phase-matched SoI sweep experiments."""

import os
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
from matplotlib.lines import Line2D
from matplotlib.ticker import MaxNLocator

from .common import (
    C_BLUE, C_CYAN, C_GREEN, C_ORANGE, C_PURPLE, C_RED,
    ext_subdir, format_number, plot_perf_metrics, save_fig,
)
from .soi import SOI_COLORS, SOI_MARKERS, _shade_gate


def _phase_label(phase_map: str) -> str:
    """Convert 'cpu:100+iobw:0' to a readable label like 'cpu@IO, iobw@CPU'."""
    parts = []
    for entry in phase_map.split('+'):
        stype, io_pct = entry.split(':')
        phase = 'IO' if int(io_pct) == 100 else ('CPU' if int(io_pct) == 0 else f'{io_pct}%io')
        parts.append(f'{stype}@{phase}')
    return ', '.join(parts)


def plot_soi_phase(csv_path: str):
    """Generate graphs for phase-matched SoI sweep CSVs."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    phase_map = df['phase_map'].iloc[0]
    label = _phase_label(phase_map)
    victim_workers = int(df['victim_workers'].iloc[0])
    x = df['soi_per_type']

    has_cpu_stddev = 'victim_cpu_stddev' in df.columns
    has_io_stddev = 'victim_io_stddev' in df.columns
    has_cpu = df['victim_cpu_ops'].max() > 0
    has_io = df['victim_io_ops'].max() > 0

    print(f"  -> {folder}/")

    # 1. Victim throughput vs SoI per type
    nplots = (1 if has_cpu else 0) + (1 if has_io else 0)
    if nplots == 0:
        nplots = 1

    if nplots == 2:
        fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 5), sharex=True,
                                             layout='constrained')
        ax_cpu.errorbar(x, df['victim_cpu_ops'],
                        yerr=df['victim_cpu_stddev'] if has_cpu_stddev else None,
                        fmt='-o', color=C_BLUE, linewidth=2, markersize=6,
                        capsize=3, capthick=1, label='Victim CPU ops/s')
        ax_cpu.set_ylabel('CPU ops/sec')
        ax_cpu.set_ylim(bottom=0)
        ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        ax_cpu.legend(loc='best', fontsize=9)
        ax_cpu.set_title(f'Phase SoI ({label}) — Victim Throughput ({victim_workers} victims)')
        ax_cpu.grid(True, alpha=0.3)

        ax_io.errorbar(x, df['victim_io_ops'],
                       yerr=df['victim_io_stddev'] if has_io_stddev else None,
                       fmt='-s', color=C_GREEN, linewidth=2, markersize=6,
                       capsize=3, capthick=1, label='Victim IO ops/s')
        ax_io.set_xlabel('SoI Workers (per type)')
        ax_io.set_ylabel('IO ops/sec')
        ax_io.set_ylim(bottom=0)
        ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        ax_io.legend(loc='best', fontsize=9)
        ax_io.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax_io.grid(True, alpha=0.3)
    else:
        fig, ax = plt.subplots(figsize=(8, 5))
        col = 'victim_cpu_ops' if has_cpu else 'victim_io_ops'
        col_label = 'Victim CPU ops/s' if has_cpu else 'Victim IO ops/s'
        stddev = df['victim_cpu_stddev'] if (has_cpu and has_cpu_stddev) else (df['victim_io_stddev'] if has_io_stddev else None)
        ax.errorbar(x, df[col], yerr=stddev, fmt='-o', color=C_BLUE,
                    linewidth=2, markersize=6, capsize=3, capthick=1, label=col_label)
        ax.set_xlabel('SoI Workers (per type)')
        ax.set_ylabel(f'{col_label} (ops/sec)')
        ax.set_title(f'Phase SoI ({label}) — Victim Throughput ({victim_workers} victims)')
        ax.set_ylim(bottom=0)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend(loc='best', fontsize=9)
        ax.grid(True, alpha=0.3)
        ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'victim_throughput.png'))

    # 2. Victim degradation
    has_cpu_change = 'victim_cpu_change_pct' in df.columns and has_cpu
    has_io_change = 'victim_io_change_pct' in df.columns and has_io
    if has_cpu_change and has_io_change:
        fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(8, 7), sharex=True, layout='constrained')
        ax_cpu.plot(x, df['victim_cpu_change_pct'], '-o', color=C_BLUE, linewidth=2, markersize=5)
        ax_cpu.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax_cpu.fill_between(x, df['victim_cpu_change_pct'], 0,
                            where=(df['victim_cpu_change_pct'] < 0), interpolate=True, alpha=0.2, color=C_RED)
        ax_cpu.set_ylabel('CPU Change (%)')
        ax_cpu.grid(True, alpha=0.3)

        ax_io.plot(x, df['victim_io_change_pct'], '-s', color=C_GREEN, linewidth=2, markersize=5)
        ax_io.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax_io.fill_between(x, df['victim_io_change_pct'], 0,
                           where=(df['victim_io_change_pct'] < 0), interpolate=True, alpha=0.2, color=C_RED)
        ax_io.set_xlabel('SoI Workers (per type)')
        ax_io.set_ylabel('IO Change (%)')
        ax_io.grid(True, alpha=0.3)
        fig.suptitle(f'Phase SoI ({label}) — Victim Degradation ({victim_workers} victims)')
    else:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['victim_change_pct'], '-o', color=C_RED, linewidth=2, markersize=5)
        ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax.fill_between(x, df['victim_change_pct'], 0,
                        where=(df['victim_change_pct'] >= 0), interpolate=True, alpha=0.2, color=C_GREEN)
        ax.fill_between(x, df['victim_change_pct'], 0,
                        where=(df['victim_change_pct'] < 0), interpolate=True, alpha=0.2, color=C_RED)
        ax.set_xlabel('SoI Workers (per type)')
        ax.set_ylabel('Victim Throughput Change (%)')
        ax.set_title(f'Phase SoI ({label}) — Victim Degradation ({victim_workers} victims)')
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'victim_degradation.png'))

    # 3. Throughput vs degradation (dual axis)
    primary_col = 'victim_io_ops' if has_io else 'victim_cpu_ops'
    primary_label = 'Victim IO ops/s' if has_io else 'Victim CPU ops/s'
    fig, ax1 = plt.subplots(figsize=(8, 5))
    line_tp = ax1.plot(x, df[primary_col], '-o', color=C_BLUE, linewidth=2,
                       markersize=6, label=primary_label)
    ax1.set_xlabel('SoI Workers (per type)')
    ax1.set_ylabel(f'{primary_label} (ops/sec)', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.set_ylim(bottom=0)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax2 = ax1.twinx()
    line_deg = ax2.plot(x, df['victim_change_pct'], '-s', color=C_RED, linewidth=2,
                        markersize=5, label='Victim change %')
    ax2.set_ylabel('Victim Change (%)', color=C_RED)
    ax2.tick_params(axis='y', labelcolor=C_RED)

    lines = line_tp + line_deg
    ax1.legend(lines, [l.get_label() for l in lines], loc='best')
    ax1.set_title(f'Phase SoI ({label}) — Throughput vs Degradation')
    ax1.set_xlim(left=0)
    ax1.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'throughput_vs_degradation.png'))

    # 4. Utilization
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO BW %')
        if 'io_iops_util_pct' in df.columns:
            ax.plot(x, df['io_iops_util_pct'], '-^', color=C_CYAN, linewidth=2, markersize=6, label='IO IOPS %')
        ax.set_xlabel('SoI Workers (per type)')
        ax.set_ylabel('Utilization (%)')
        ax.set_title(f'Phase SoI ({label}) — Resource Utilization')
        ax.set_ylim(0, 100)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'utilization.png'))

    # 5. Throughput vs utilization (dual axis, split CPU/IO)
    if 'cpu_pct' in df.columns and has_cpu and has_io:
        has_io_util = 'io_util_pct' in df.columns
        has_iops_util = 'io_iops_util_pct' in df.columns

        fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 6), sharex=True,
                                             layout='constrained')

        ax_cpu.errorbar(x, df['victim_cpu_ops'],
                        yerr=df['victim_cpu_stddev'] if has_cpu_stddev else None,
                        fmt='-o', color=C_BLUE, linewidth=2, markersize=5,
                        capsize=3, capthick=1, label='Victim CPU ops/s')
        ax_cpu.set_ylabel('CPU ops/sec', color=C_BLUE)
        ax_cpu.tick_params(axis='y', labelcolor=C_BLUE)
        ax_cpu.set_ylim(bottom=0)
        ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        ax_cpu.grid(True, alpha=0.3)

        ax_cpu2 = ax_cpu.twinx()
        ax_cpu2.plot(x, df['cpu_pct'], '--s', color=C_ORANGE, linewidth=1.5,
                     markersize=4, alpha=0.8, label='CPU util %')
        ax_cpu2.set_ylabel('CPU Utilization (%)', color=C_ORANGE)
        ax_cpu2.tick_params(axis='y', labelcolor=C_ORANGE)
        ax_cpu2.set_ylim(0, 105)

        handles_cpu = [Line2D([], [], color=C_BLUE, marker='o', markersize=5, label='Victim CPU ops/s'),
                       Line2D([], [], color=C_ORANGE, marker='s', markersize=4, linestyle='--', alpha=0.8, label='CPU util %')]
        ax_cpu.legend(handles=handles_cpu, loc='best', fontsize=9)
        ax_cpu.set_title(f'Phase SoI ({label}) — Throughput vs Utilization ({victim_workers} victims)')

        ax_io.errorbar(x, df['victim_io_ops'],
                       yerr=df['victim_io_stddev'] if has_io_stddev else None,
                       fmt='-o', color=C_GREEN, linewidth=2, markersize=5,
                       capsize=3, capthick=1, label='Victim IO ops/s')
        ax_io.set_ylabel('IO ops/sec', color=C_GREEN)
        ax_io.tick_params(axis='y', labelcolor=C_GREEN)
        ax_io.set_ylim(bottom=0)
        ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        ax_io.set_xlabel('SoI Workers (per type)')
        ax_io.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax_io.grid(True, alpha=0.3)

        ax_io2 = ax_io.twinx()
        handles_io = [Line2D([], [], color=C_GREEN, marker='o', markersize=5, label='Victim IO ops/s')]
        if has_io_util:
            ax_io2.plot(x, df['io_util_pct'], '--s', color=C_CYAN, linewidth=1.5,
                        markersize=4, alpha=0.8, label='IO BW util %')
            handles_io.append(Line2D([], [], color=C_CYAN, marker='s', markersize=4, linestyle='--', alpha=0.8, label='IO BW util %'))
        if has_iops_util:
            ax_io2.plot(x, df['io_iops_util_pct'], '--^', color=C_PURPLE, linewidth=1.5,
                        markersize=4, alpha=0.8, label='IO IOPS util %')
            handles_io.append(Line2D([], [], color=C_PURPLE, marker='^', markersize=4, linestyle='--', alpha=0.8, label='IO IOPS util %'))
        ax_io2.set_ylabel('IO Utilization (%)')
        ax_io2.set_ylim(0, 105)
        ax_io.legend(handles=handles_io, loc='best', fontsize=9)

        save_fig(fig, os.path.join(folder, 'throughput_vs_utilization.png'))

    # 6. PSI pressure
    if 'psi_cpu_some_us' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        if 'io_psi_pct' in df.columns:
            ax.plot(x, df['io_psi_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO PSI %')
        ax.set_xlabel('SoI Workers (per type)')
        ax.set_ylabel('PSI Pressure (%)')
        ax.set_title(f'Phase SoI ({label}) — Pressure Stall Information')
        ax.set_ylim(bottom=0)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'psi_pressure.png'))

    # 7. SoI throughput (aggregate)
    has_soi_cpu_agg = 'soi_cpu_ops' in df.columns and df['soi_cpu_ops'].max() > 0
    has_soi_io_agg = 'soi_io_ops' in df.columns and df['soi_io_ops'].max() > 0
    if 'soi_ops' in df.columns and df['soi_ops'].max() > 0:
        if has_soi_cpu_agg and has_soi_io_agg:
            fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(8, 7), sharex=True,
                                                 layout='constrained')
            ax_cpu.plot(x, df['soi_cpu_ops'], '-^', color=C_BLUE, linewidth=2, markersize=6, label='SoI CPU ops/s')
            ax_cpu.set_ylabel('SoI CPU ops/sec')
            ax_cpu.set_ylim(bottom=0)
            ax_cpu.legend(loc='best', fontsize=9)
            ax_cpu.grid(True, alpha=0.3)
            ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
            ax_cpu.set_title(f'Phase SoI ({label}) — SoI Throughput')

            ax_io.plot(x, df['soi_io_ops'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='SoI IO ops/s')
            ax_io.set_xlabel('SoI Workers (per type)')
            ax_io.set_ylabel('SoI IO ops/sec')
            ax_io.set_ylim(bottom=0)
            ax_io.xaxis.set_major_locator(MaxNLocator(integer=True))
            ax_io.legend(loc='best', fontsize=9)
            ax_io.grid(True, alpha=0.3)
            ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        else:
            fig, ax = plt.subplots(figsize=(8, 5))
            if has_soi_cpu_agg:
                ax.plot(x, df['soi_cpu_ops'], '-^', color=C_BLUE, linewidth=2, markersize=6, label='SoI CPU ops/s')
            elif has_soi_io_agg:
                ax.plot(x, df['soi_io_ops'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='SoI IO ops/s')
            else:
                ax.plot(x, df['soi_ops'], '-^', color=C_ORANGE, linewidth=2, markersize=6, label='SoI ops/s')
            ax.set_xlabel('SoI Workers (per type)')
            ax.set_ylabel('SoI Throughput (ops/sec)')
            ax.set_title(f'Phase SoI ({label}) — SoI Throughput')
            ax.set_ylim(bottom=0)
            ax.set_xlim(left=0)
            ax.xaxis.set_major_locator(MaxNLocator(integer=True))
            ax.legend(loc='best', fontsize=9)
            ax.grid(True, alpha=0.3)
            ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        save_fig(fig, os.path.join(folder, 'soi_throughput.png'))

    # 8. Hardware perf counters
    plot_perf_metrics(df, 'soi_per_type', 'SoI Workers (per type)',
                      f'Phase SoI ({label})', folder,
                      throughput_col='victim_total_ops')


def plot_per_worker_soi_phase(csv_path: str):
    """Box plot of victim worker throughput distribution as phase SoI workers increase."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    soi_counts = sorted(df['soi_per_type'].unique())
    victim_df = df[df['role'] == 'victim']

    print(f"  -> {folder}/")

    box_width = (soi_counts[1] - soi_counts[0]) * 0.6 if len(soi_counts) > 1 else 0.6

    fig, ax = plt.subplots(figsize=(max(8, len(soi_counts) * 0.5), 5))
    data = [victim_df[victim_df['soi_per_type'] == s]['total_ops_sec'].values for s in soi_counts]
    ax.boxplot(data, positions=soi_counts, widths=box_width * 0.7, patch_artist=True,
               boxprops=dict(facecolor=C_CYAN, alpha=0.7),
               medianprops=dict(color=C_BLUE, linewidth=2), manage_ticks=False)
    ax.set_xticks(soi_counts)
    ax.set_xticklabels(soi_counts)
    ax.set_xlabel('SoI Workers (per type)')
    ax.set_ylabel('Per-Worker Throughput (ops/sec)')
    ax.set_title('Phase SoI — Victim Worker Distribution')
    ax.set_ylim(bottom=0)
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'per_worker_distribution.png'))

    totals = [victim_df[victim_df['soi_per_type'] == s]['total_ops_sec'].sum() for s in soi_counts]
    cvs = []
    for s in soi_counts:
        vals = victim_df[victim_df['soi_per_type'] == s]['total_ops_sec'].values
        mean = vals.mean()
        cvs.append(vals.std() / mean * 100 if mean > 0 else 0)

    fig, ax1 = plt.subplots(figsize=(8, 5))
    line_tp = ax1.plot(soi_counts, totals, '-o', color=C_BLUE, linewidth=2, markersize=6, label='Victim total throughput')
    ax1.set_xlabel('SoI Workers (per type)')
    ax1.set_ylabel('Victim Total Throughput (ops/sec)', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.set_ylim(bottom=0)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax2 = ax1.twinx()
    line_cv = ax2.plot(soi_counts, cvs, '-s', color=C_RED, linewidth=2, markersize=5, label='Victim fairness CV%')
    ax2.set_ylabel('Coefficient of Variation (%)', color=C_RED)
    ax2.tick_params(axis='y', labelcolor=C_RED)
    ax2.set_ylim(bottom=0)

    lines = line_tp + line_cv
    ax1.legend(lines, [l.get_label() for l in lines], loc='best')
    ax1.set_title('Phase SoI — Victim Throughput vs Fairness')
    ax1.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'per_worker_fairness.png'))

    # Per-type SoI throughput breakdown
    soi_df = df[df['role'] == 'soi']
    if len(soi_df) > 0:
        soi_types = sorted(soi_df['soi_type'].dropna().unique())
        if soi_types:
            fig, ax = plt.subplots(figsize=(8, 5))
            for stype in soi_types:
                type_df = soi_df[soi_df['soi_type'] == stype]
                means = []
                counts = []
                for s in soi_counts:
                    vals = type_df[type_df['soi_per_type'] == s]['total_ops_sec'].values
                    means.append(vals.mean() if len(vals) else 0)
                    counts.append(s)
                color = SOI_COLORS.get(stype, 'gray')
                marker = SOI_MARKERS.get(stype, 'o')
                ax.plot(counts, means, f'-{marker}', color=color, linewidth=2,
                        markersize=6, label=stype)
            ax.set_xlabel('SoI Workers (per type)')
            ax.set_ylabel('Mean SoI Worker Throughput (ops/sec)')
            ax.set_title('Phase SoI — SoI Worker Throughput by Type')
            ax.set_ylim(bottom=0)
            ax.xaxis.set_major_locator(MaxNLocator(integer=True))
            ax.legend(loc='best', fontsize=9)
            ax.grid(True, alpha=0.3)
            ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
            save_fig(fig, os.path.join(folder, 'soi_throughput_by_type.png'))


def plot_soi_phase_timeseries(csv_path: str):
    """Generate time-series plots for phase-matched SoI sweep."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    soi_counts = sorted(df['soi_per_type'].unique())
    has_victim_gate = 'victim_gate_on' in df.columns
    has_multi_sample = 'sample_idx' in df.columns
    has_io_phase = 'victim_io_phase' in df.columns

    has_cpu = df['victim_cpu_ops_sec'].max() > 0 if 'victim_cpu_ops_sec' in df.columns else False
    has_io = df['victim_io_ops_sec'].max() > 0 if 'victim_io_ops_sec' in df.columns else False
    has_both = has_cpu and has_io
    tp_col = 'victim_cpu_ops_sec' if has_cpu else 'victim_io_ops_sec'
    tp_label = 'Victim CPU ops/s' if has_cpu else 'Victim IO ops/s'

    n_samples = df['sample_idx'].nunique() if has_multi_sample else 1
    group_cols = ['soi_per_type', 'elapsed_ms']

    if has_multi_sample and n_samples > 1:
        agg = df.groupby(group_cols).agg(
            tp_mean=(tp_col, 'mean'), tp_std=(tp_col, 'std'),
            **({'cpu_mean': ('victim_cpu_ops_sec', 'mean'), 'cpu_std': ('victim_cpu_ops_sec', 'std')} if has_both else {}),
            **({'io_mean': ('victim_io_ops_sec', 'mean'), 'io_std': ('victim_io_ops_sec', 'std')} if has_both else {}),
            cpu_pct_mean=('cpu_pct', 'mean') if 'cpu_pct' in df.columns else (tp_col, 'count'),
            **({'io_util_mean': ('io_util_pct', 'mean')} if 'io_util_pct' in df.columns else {}),
            **({'io_iops_mean': ('io_iops_util_pct', 'mean')} if 'io_iops_util_pct' in df.columns else {}),
            **({'mem_mean': ('mem_usage_pct', 'mean')} if 'mem_usage_pct' in df.columns else {}),
            **({'victim_gate_on': ('victim_gate_on', 'mean')} if has_victim_gate else {}),
            **({'victim_io_phase': ('victim_io_phase', 'mean')} if has_io_phase else {}),
        ).reset_index()
        agg['tp_std'] = agg['tp_std'].fillna(0)
        if has_both:
            agg['cpu_std'] = agg['cpu_std'].fillna(0)
            agg['io_std'] = agg['io_std'].fillna(0)
    else:
        agg = df.rename(columns={tp_col: 'tp_mean'}).copy()
        agg['tp_std'] = 0.0
        if has_both:
            agg['cpu_mean'] = df['victim_cpu_ops_sec']
            agg['cpu_std'] = 0.0
            agg['io_mean'] = df['victim_io_ops_sec']
            agg['io_std'] = 0.0
        if 'cpu_pct' in df.columns:
            agg['cpu_pct_mean'] = agg['cpu_pct']
        if 'io_util_pct' in df.columns:
            agg['io_util_mean'] = agg['io_util_pct']
        if 'io_iops_util_pct' in df.columns:
            agg['io_iops_mean'] = agg['io_iops_util_pct']
        if 'mem_usage_pct' in df.columns:
            agg['mem_mean'] = agg['mem_usage_pct']

    print(f"  -> {folder}/ (timeseries, {n_samples} sample(s))")

    ts_dir = ext_subdir(folder, 'timeseries')

    # 1. Throughput time-series — all SoI counts overlaid
    title_suffix = f' (mean +/- sigma, n={n_samples})' if n_samples > 1 else ''
    if has_both:
        fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 8), sharex=True, layout='constrained')
        cmap = plt.cm.viridis
        for i, s in enumerate(soi_counts):
            sub = agg[agg['soi_per_type'] == s].sort_values('elapsed_ms')
            color = cmap(i / max(len(soi_counts) - 1, 1))
            t = sub['elapsed_ms'] / 1000
            ax_cpu.plot(t, sub['cpu_mean'], '-', color=color, linewidth=1.2, alpha=0.8, label=f'{int(s)}/type')
            ax_io.plot(t, sub['io_mean'], '-', color=color, linewidth=1.2, alpha=0.8, label=f'{int(s)}/type')
            if n_samples > 1:
                ax_cpu.fill_between(t, sub['cpu_mean'] - sub['cpu_std'], sub['cpu_mean'] + sub['cpu_std'],
                                    color=color, alpha=0.15)
                ax_io.fill_between(t, sub['io_mean'] - sub['io_std'], sub['io_mean'] + sub['io_std'],
                                   color=color, alpha=0.15)
        ax_cpu.set_ylabel('CPU ops/sec')
        ax_cpu.set_ylim(bottom=0)
        ax_cpu.set_title(f'Phase SoI — Victim Throughput Time Series{title_suffix}')
        ax_cpu.legend(loc='best', fontsize=8, ncol=2)
        ax_cpu.grid(True, alpha=0.3)
        ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        ax_io.set_xlabel('Elapsed (seconds)')
        ax_io.set_ylabel('IO ops/sec')
        ax_io.set_ylim(bottom=0)
        ax_io.legend(loc='best', fontsize=8, ncol=2)
        ax_io.grid(True, alpha=0.3)
        ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    else:
        fig, ax = plt.subplots(figsize=(10, 6))
        cmap = plt.cm.viridis
        for i, s in enumerate(soi_counts):
            sub = agg[agg['soi_per_type'] == s].sort_values('elapsed_ms')
            color = cmap(i / max(len(soi_counts) - 1, 1))
            t = sub['elapsed_ms'] / 1000
            ax.plot(t, sub['tp_mean'], '-', color=color, linewidth=1.2, alpha=0.8, label=f'{int(s)}/type')
            if n_samples > 1:
                ax.fill_between(t, sub['tp_mean'] - sub['tp_std'], sub['tp_mean'] + sub['tp_std'],
                                color=color, alpha=0.15)
        ax.set_xlabel('Elapsed (seconds)')
        ax.set_ylabel(tp_label)
        ax.set_title(f'Phase SoI — Victim Throughput Time Series{title_suffix}')
        ax.set_ylim(bottom=0)
        ax.legend(loc='best', fontsize=8, ncol=2)
        ax.grid(True, alpha=0.3)
        ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(ts_dir, 'victim_throughput.png'))

    # 2. Heatmap
    if len(soi_counts) > 1:
        pivot = agg.pivot_table(index='soi_per_type', columns='elapsed_ms', values='tp_mean')
        fig, ax = plt.subplots(figsize=(12, 5))
        im = ax.imshow(pivot.values, aspect='auto', cmap='viridis',
                       extent=[pivot.columns.min() / 1000, pivot.columns.max() / 1000,
                               pivot.index.max() + 0.5, pivot.index.min() - 0.5])
        ax.set_xlabel('Elapsed (seconds)')
        ax.set_ylabel('SoI Workers (per type)')
        ax.set_title('Phase SoI — Throughput Heatmap')
        ax.set_yticks(soi_counts)
        cbar = fig.colorbar(im, ax=ax, label=tp_label)
        cbar.formatter = plt.FuncFormatter(lambda v, p: format_number(v))
        cbar.update_ticks()
        save_fig(fig, os.path.join(ts_dir, 'heatmap.png'))

    # 3. Per-SoI-count detail: throughput + utilization + phase shading
    has_cpu_pct = 'cpu_pct_mean' in agg.columns
    has_io_bw = 'io_util_mean' in agg.columns
    has_io_iops = 'io_iops_mean' in agg.columns
    has_mem = 'mem_mean' in agg.columns

    for g in soi_counts:
        sub = agg[agg['soi_per_type'] == g].sort_values('elapsed_ms')
        if sub.empty:
            continue

        t = sub['elapsed_ms'] / 1000

        def _apply_phase_shading(ax):
            if has_victim_gate and 'victim_gate_on' in sub.columns:
                gate_sub = sub.copy()
                gate_sub['victim_gate_on'] = (gate_sub['victim_gate_on'] >= 0.5).astype(int)
                _shade_gate(ax, t, 'victim_gate_on', gate_sub, 'gray', 'Victim OFF')
            if has_io_phase and 'victim_io_phase' in sub.columns:
                io_phase = sub['victim_io_phase'].values
                for i in range(len(io_phase)):
                    if io_phase[i] > 0.5:
                        x0 = t.iloc[max(0, i - 1)] if i > 0 else t.iloc[0]
                        x1 = t.iloc[i]
                        ax.axvspan(x0, x1, color=C_GREEN, alpha=0.06)

        if has_both:
            fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(11, 7), sharex=True,
                                                 layout='constrained')
            _apply_phase_shading(ax_cpu)
            _apply_phase_shading(ax_io)

            line_cpu, = ax_cpu.plot(t, sub['cpu_mean'], '-', color=C_BLUE, linewidth=2.2, label='CPU ops/s')
            if n_samples > 1:
                ax_cpu.fill_between(t, sub['cpu_mean'] - sub['cpu_std'], sub['cpu_mean'] + sub['cpu_std'],
                                    color=C_BLUE, alpha=0.15)
            ax_cpu.set_ylabel('CPU ops/sec', color=C_BLUE)
            ax_cpu.tick_params(axis='y', labelcolor=C_BLUE)
            ax_cpu.set_ylim(bottom=0)
            ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
            ax_cpu.grid(True, alpha=0.3)
            cpu_lines = [line_cpu]
            if has_cpu_pct:
                ax_cpu2 = ax_cpu.twinx()
                l, = ax_cpu2.plot(t, sub['cpu_pct_mean'], '--', color=C_ORANGE,
                                  linewidth=1.2, alpha=0.8, label='CPU%')
                ax_cpu2.set_ylabel('CPU Util (%)', color=C_ORANGE)
                ax_cpu2.tick_params(axis='y', labelcolor=C_ORANGE)
                ax_cpu2.set_ylim(0, 105)
                cpu_lines.append(l)
            ax_cpu.legend(cpu_lines, [l.get_label() for l in cpu_lines], loc='upper right', fontsize=9)

            line_io, = ax_io.plot(t, sub['io_mean'], '-', color=C_GREEN, linewidth=2.2, label='IO ops/s')
            if n_samples > 1:
                ax_io.fill_between(t, sub['io_mean'] - sub['io_std'], sub['io_mean'] + sub['io_std'],
                                   color=C_GREEN, alpha=0.15)
            ax_io.set_ylabel('IO ops/sec', color=C_GREEN)
            ax_io.tick_params(axis='y', labelcolor=C_GREEN)
            ax_io.set_ylim(bottom=0)
            ax_io.set_xlabel('Elapsed (seconds)')
            ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
            ax_io.grid(True, alpha=0.3)
            io_lines = [line_io]
            if has_io_bw or has_io_iops:
                ax_io2 = ax_io.twinx()
                ax_io2.set_ylim(0, 105)
                if has_io_bw:
                    l, = ax_io2.plot(t, sub['io_util_mean'], '--', color=C_CYAN,
                                     linewidth=1.2, alpha=0.8, label='IO BW%')
                    io_lines.append(l)
                if has_io_iops:
                    l, = ax_io2.plot(t, sub['io_iops_mean'], '--', color=C_PURPLE,
                                     linewidth=1.2, alpha=0.8, label='IO IOPS%')
                    io_lines.append(l)
                ax_io2.set_ylabel('IO Util (%)')
            ax_io.legend(io_lines, [l.get_label() for l in io_lines], loc='upper right', fontsize=9)

            fig.suptitle(f'Phase SoI — SoI/type={int(g)}')
        else:
            fig, ax_tp = plt.subplots(figsize=(11, 5))
            _apply_phase_shading(ax_tp)
            line_tp, = ax_tp.plot(t, sub['tp_mean'], '-', color=C_BLUE, linewidth=2.2, label=tp_label)
            if n_samples > 1:
                ax_tp.fill_between(t, sub['tp_mean'] - sub['tp_std'], sub['tp_mean'] + sub['tp_std'],
                                   color=C_BLUE, alpha=0.2, label=f'+/- sigma (n={n_samples})')
            ax_tp.set_xlabel('Elapsed (seconds)')
            ax_tp.set_ylabel(tp_label, color=C_BLUE)
            ax_tp.tick_params(axis='y', labelcolor=C_BLUE)
            ax_tp.set_ylim(bottom=0)
            ax_tp.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
            ax_tp.grid(True, alpha=0.3)
            ax_util = ax_tp.twinx()
            lines = [line_tp]
            if has_cpu_pct:
                l, = ax_util.plot(t, sub['cpu_pct_mean'], '--', color=C_ORANGE,
                                  linewidth=1.2, alpha=0.8, label='CPU%')
                lines.append(l)
            if has_io_bw:
                l, = ax_util.plot(t, sub['io_util_mean'], '--', color=C_GREEN,
                                  linewidth=1.2, alpha=0.8, label='IO BW%')
                lines.append(l)
            if has_io_iops:
                l, = ax_util.plot(t, sub['io_iops_mean'], '--', color=C_PURPLE,
                                  linewidth=1.2, alpha=0.8, label='IO IOPS%')
                lines.append(l)
            if has_mem:
                l, = ax_util.plot(t, sub['mem_mean'], '--', color=C_RED,
                                  linewidth=1.2, alpha=0.8, label='Mem%')
                lines.append(l)
            ax_util.set_ylabel('Utilization (%)')
            ax_util.set_ylim(0, 105)
            ax_tp.set_title(f'Phase SoI — SoI/type={int(g)}')
            ax_tp.legend(lines, [l.get_label() for l in lines], loc='upper right', fontsize=9)
            fig.tight_layout()

        save_fig(fig, os.path.join(ts_dir, f'timeseries_per_soi_{int(g):02d}.png'))

    # 4. SoI throughput time-series — one graph per SoI count
    has_soi_split = 'soi_cpu_ops_sec' in df.columns and 'soi_io_ops_sec' in df.columns
    if 'soi_ops_sec' in df.columns:
        soi_df = df[df['soi_per_type'] > 0]
        if len(soi_df) and soi_df['soi_ops_sec'].max() > 0:
            soi_nonzero = sorted(soi_df['soi_per_type'].unique())

            if has_multi_sample and n_samples > 1:
                soi_agg = soi_df.groupby(group_cols).agg(
                    soi_mean=('soi_ops_sec', 'mean'), soi_std=('soi_ops_sec', 'std'),
                    **({'soi_gate_on': ('soi_gate_on', 'mean')} if 'soi_gate_on' in soi_df.columns else {}),
                    **({'soi_cpu_mean': ('soi_cpu_ops_sec', 'mean'), 'soi_cpu_std': ('soi_cpu_ops_sec', 'std')} if has_soi_split else {}),
                    **({'soi_io_mean': ('soi_io_ops_sec', 'mean'), 'soi_io_std': ('soi_io_ops_sec', 'std')} if has_soi_split else {}),
                ).reset_index()
                soi_agg['soi_std'] = soi_agg['soi_std'].fillna(0)
                if has_soi_split:
                    soi_agg['soi_cpu_std'] = soi_agg['soi_cpu_std'].fillna(0)
                    soi_agg['soi_io_std'] = soi_agg['soi_io_std'].fillna(0)
            else:
                soi_agg = soi_df.rename(columns={'soi_ops_sec': 'soi_mean'}).copy()
                soi_agg['soi_std'] = 0.0
                if has_soi_split:
                    soi_agg['soi_cpu_mean'] = soi_df['soi_cpu_ops_sec']
                    soi_agg['soi_cpu_std'] = 0.0
                    soi_agg['soi_io_mean'] = soi_df['soi_io_ops_sec']
                    soi_agg['soi_io_std'] = 0.0

            has_soi_cpu = has_soi_split and soi_df['soi_cpu_ops_sec'].max() > 0
            has_soi_io = has_soi_split and soi_df['soi_io_ops_sec'].max() > 0
            has_soi_both = has_soi_cpu and has_soi_io

            for s in soi_nonzero:
                sub = soi_agg[soi_agg['soi_per_type'] == s].sort_values('elapsed_ms')
                if sub.empty:
                    continue
                t = sub['elapsed_ms'] / 1000

                if has_soi_both:
                    fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(11, 7), sharex=True,
                                                         layout='constrained')
                    if 'soi_gate_on' in sub.columns:
                        gate_sub = sub.copy()
                        gate_sub['soi_gate_on'] = (gate_sub['soi_gate_on'] >= 0.5).astype(int)
                        _shade_gate(ax_cpu, t, 'soi_gate_on', gate_sub, C_ORANGE, 'SoI OFF')
                        _shade_gate(ax_io, t, 'soi_gate_on', gate_sub, C_ORANGE, 'SoI OFF')
                    ax_cpu.plot(t, sub['soi_cpu_mean'], '-', color=C_BLUE, linewidth=2.2, label='SoI CPU ops/s')
                    if n_samples > 1:
                        ax_cpu.fill_between(t, sub['soi_cpu_mean'] - sub['soi_cpu_std'],
                                            sub['soi_cpu_mean'] + sub['soi_cpu_std'],
                                            color=C_BLUE, alpha=0.2, label=f'±σ (n={n_samples})')
                    ax_cpu.set_ylabel('SoI CPU ops/sec')
                    ax_cpu.set_ylim(bottom=0)
                    ax_cpu.legend(loc='best', fontsize=9)
                    ax_cpu.grid(True, alpha=0.3)
                    ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

                    ax_io.plot(t, sub['soi_io_mean'], '-', color=C_GREEN, linewidth=2.2, label='SoI IO ops/s')
                    if n_samples > 1:
                        ax_io.fill_between(t, sub['soi_io_mean'] - sub['soi_io_std'],
                                           sub['soi_io_mean'] + sub['soi_io_std'],
                                           color=C_GREEN, alpha=0.2, label=f'±σ (n={n_samples})')
                    ax_io.set_xlabel('Elapsed (seconds)')
                    ax_io.set_ylabel('SoI IO ops/sec')
                    ax_io.set_ylim(bottom=0)
                    ax_io.legend(loc='best', fontsize=9)
                    ax_io.grid(True, alpha=0.3)
                    ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
                    fig.suptitle(f'Phase SoI — SoI Throughput, SoI/type={int(s)}')
                else:
                    fig, ax = plt.subplots(figsize=(11, 5))
                    if 'soi_gate_on' in sub.columns:
                        gate_sub = sub.copy()
                        gate_sub['soi_gate_on'] = (gate_sub['soi_gate_on'] >= 0.5).astype(int)
                        _shade_gate(ax, t, 'soi_gate_on', gate_sub, C_ORANGE, 'SoI OFF')
                    col = 'soi_cpu_mean' if has_soi_cpu else ('soi_io_mean' if has_soi_io else 'soi_mean')
                    std_col = 'soi_cpu_std' if has_soi_cpu else ('soi_io_std' if has_soi_io else 'soi_std')
                    lbl = 'SoI CPU ops/s' if has_soi_cpu else ('SoI IO ops/s' if has_soi_io else 'SoI ops/s')
                    clr = C_BLUE if has_soi_cpu else (C_GREEN if has_soi_io else C_ORANGE)
                    ax.plot(t, sub[col], '-', color=clr, linewidth=2.2, label=lbl)
                    if n_samples > 1:
                        ax.fill_between(t, sub[col] - sub[std_col],
                                        sub[col] + sub[std_col],
                                        color=clr, alpha=0.2, label=f'±σ (n={n_samples})')
                    ax.set_xlabel('Elapsed (seconds)')
                    ax.set_ylabel('SoI Throughput (ops/sec)')
                    ax.set_title(f'Phase SoI — SoI Throughput, SoI/type={int(s)}')
                    ax.set_ylim(bottom=0)
                    ax.legend(loc='best', fontsize=9)
                    ax.grid(True, alpha=0.3)
                    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
                    fig.tight_layout()
                save_fig(fig, os.path.join(ts_dir, f'soi_throughput_per_soi_{int(s):02d}.png'))

    # 5. Gate-aware throughput comparison (victim phases)
    if has_io_phase:
        cpu_means = []
        io_means = []
        for s in soi_counts:
            sub = df[df['soi_per_type'] == s]
            cpu_data = sub[sub['victim_io_phase'] < 0.5]['victim_cpu_ops_sec']
            io_data = sub[sub['victim_io_phase'] >= 0.5]['victim_io_ops_sec']
            cpu_means.append(cpu_data.mean() if len(cpu_data) else 0)
            io_means.append(io_data.mean() if len(io_data) else 0)

        fig, ax = plt.subplots(figsize=(10, 5))
        x_pos = np.arange(len(soi_counts))
        w = 0.35
        ax.bar(x_pos - w / 2, cpu_means, w, color=C_BLUE, alpha=0.8, label='CPU phase')
        ax.bar(x_pos + w / 2, io_means, w, color=C_GREEN, alpha=0.8, label='IO phase')
        ax.set_xticks(x_pos)
        ax.set_xticklabels([str(s) for s in soi_counts])
        ax.set_xlabel('SoI Workers (per type)')
        ax.set_ylabel('Victim Throughput (ops/sec)')
        ax.set_title('Phase SoI — Throughput by Victim Phase')
        ax.set_ylim(bottom=0)
        ax.legend(loc='best')
        ax.grid(True, alpha=0.3, axis='y')
        ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        save_fig(fig, os.path.join(ts_dir, 'phase_comparison.png'))
    elif has_victim_gate:
        on_means = []
        off_means = []
        for s in soi_counts:
            sub = df[df['soi_per_type'] == s]
            on_data = sub[sub['victim_gate_on'] == 1][tp_col]
            off_data = sub[sub['victim_gate_on'] == 0][tp_col]
            on_means.append(on_data.mean() if len(on_data) else 0)
            off_means.append(off_data.mean() if len(off_data) else 0)

        fig, ax = plt.subplots(figsize=(10, 5))
        x_pos = np.arange(len(soi_counts))
        w = 0.35
        ax.bar(x_pos - w / 2, on_means, w, color=C_BLUE, alpha=0.8, label='Victim ON')
        ax.bar(x_pos + w / 2, off_means, w, color=C_RED, alpha=0.8, label='Victim OFF')
        ax.set_xticks(x_pos)
        ax.set_xticklabels([str(s) for s in soi_counts])
        ax.set_xlabel('SoI Workers (per type)')
        ax.set_ylabel(tp_label)
        ax.set_title('Phase SoI — Throughput by Victim Gate Phase')
        ax.set_ylim(bottom=0)
        ax.legend(loc='best')
        ax.grid(True, alpha=0.3, axis='y')
        ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        save_fig(fig, os.path.join(ts_dir, 'gate_phase_comparison.png'))
