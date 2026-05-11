"""Plots for synthetic SoI (Service of Interest) sweep experiments."""

import os
from pathlib import Path

import matplotlib.pyplot as plt
import pandas as pd
from matplotlib.lines import Line2D
from matplotlib.ticker import MaxNLocator

from .common import (
    C_BLUE, C_CYAN, C_GREEN, C_ORANGE, C_PURPLE, C_RED,
    format_number, save_fig,
)


def plot_soi(csv_path: str):
    """Generate graphs for SoI (Service of Interest) sweep CSVs."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    soi_type = df['soi_type'].iloc[0]
    if soi_type == 'ioops':
        soi_type = 'iops'
    victim_workers = int(df['victim_workers'].iloc[0])
    x = df['soi_workers']

    has_cpu_stddev = 'victim_cpu_stddev' in df.columns
    has_io_stddev = 'victim_io_stddev' in df.columns

    if 'victim_cpu_change_pct' not in df.columns and 'victim_cpu_ops' in df.columns:
        base_cpu = df['victim_cpu_ops'].iloc[0]
        base_io = df['victim_io_ops'].iloc[0]
        df['victim_cpu_change_pct'] = ((df['victim_cpu_ops'] - base_cpu) / base_cpu * 100.0) if base_cpu > 0 else 0.0
        df['victim_io_change_pct'] = ((df['victim_io_ops'] - base_io) / base_io * 100.0) if base_io > 0 else 0.0

    print(f"  -> {folder}/")

    has_cpu = df['victim_cpu_ops'].max() > 0
    has_io = df['victim_io_ops'].max() > 0
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
        ax_cpu.set_title(f'SoI Sweep ({soi_type}) — Victim Throughput ({victim_workers} victims)')
        ax_cpu.grid(True, alpha=0.3)

        ax_io.errorbar(x, df['victim_io_ops'],
                       yerr=df['victim_io_stddev'] if has_io_stddev else None,
                       fmt='-s', color=C_GREEN, linewidth=2, markersize=6,
                       capsize=3, capthick=1, label='Victim IO ops/s')
        ax_io.set_xlabel('SoI Workers')
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
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel(f'{col_label} (ops/sec)')
        ax.set_title(f'SoI Sweep ({soi_type}) — Victim Throughput ({victim_workers} victims)')
        ax.set_ylim(bottom=0)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend(loc='best', fontsize=9)
        ax.grid(True, alpha=0.3)
        ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'victim_throughput.png'))

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
        ax_io.set_xlabel('SoI Workers')
        ax_io.set_ylabel('IO Change (%)')
        ax_io.grid(True, alpha=0.3)
        fig.suptitle(f'SoI Sweep ({soi_type}) — Victim Degradation ({victim_workers} victims)')
    else:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['victim_change_pct'], '-o', color=C_RED, linewidth=2, markersize=5)
        ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax.fill_between(x, df['victim_change_pct'], 0,
                        where=(df['victim_change_pct'] >= 0), interpolate=True, alpha=0.2, color=C_GREEN)
        ax.fill_between(x, df['victim_change_pct'], 0,
                        where=(df['victim_change_pct'] < 0), interpolate=True, alpha=0.2, color=C_RED)
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('Victim Throughput Change (%)')
        ax.set_title(f'SoI Sweep ({soi_type}) — Victim Degradation ({victim_workers} victims)')
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'victim_degradation.png'))

    if df['soi_ops'].max() > 0:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['soi_ops'], '-^', color=C_ORANGE, linewidth=2, markersize=6, label='SoI ops/s')
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

    primary_col = 'victim_io_ops' if has_io else 'victim_cpu_ops'
    primary_label = 'Victim IO ops/s' if has_io else 'Victim CPU ops/s'
    fig, ax1 = plt.subplots(figsize=(8, 5))
    line_tp = ax1.plot(x, df[primary_col], '-o', color=C_BLUE, linewidth=2,
                       markersize=6, label=primary_label)
    ax1.set_xlabel('SoI Workers')
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
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('Utilization (%)')
        ax.set_title(f'SoI Sweep ({soi_type}) — Resource Utilization')
        ax.set_ylim(0, 100)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'utilization.png'))

    has_psi = 'psi_cpu_some_us' in df.columns
    if has_psi:
        fig, ax = plt.subplots(figsize=(8, 5))
        if 'io_psi_pct' in df.columns:
            ax.plot(x, df['io_psi_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO PSI %')
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('PSI Pressure (%)')
        ax.set_title(f'SoI Sweep ({soi_type}) — Pressure Stall Information')
        ax.set_ylim(bottom=0)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'psi_pressure.png'))

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
        ax_cpu.set_title(f'SoI Sweep ({soi_type}) — Throughput vs Utilization ({victim_workers} victims)')

        ax_io.errorbar(x, df['victim_io_ops'],
                       yerr=df['victim_io_stddev'] if has_io_stddev else None,
                       fmt='-o', color=C_GREEN, linewidth=2, markersize=5,
                       capsize=3, capthick=1, label='Victim IO ops/s')
        ax_io.set_ylabel('IO ops/sec', color=C_GREEN)
        ax_io.tick_params(axis='y', labelcolor=C_GREEN)
        ax_io.set_ylim(bottom=0)
        ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        ax_io.set_xlabel('SoI Workers')
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


def plot_per_worker_soi(csv_path: str):
    """Box plot of victim worker throughput distribution as SoI workers increase."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    soi_counts = sorted(df['soi_workers'].unique())
    victim_df = df[~df['is_soi']]

    print(f"  -> {folder}/")

    box_width = (soi_counts[1] - soi_counts[0]) * 0.6 if len(soi_counts) > 1 else 0.6

    fig, ax = plt.subplots(figsize=(max(8, len(soi_counts) * 0.5), 5))
    data = [victim_df[victim_df['soi_workers'] == s]['total_ops_sec'].values for s in soi_counts]
    ax.boxplot(data, positions=soi_counts, widths=box_width * 0.7, patch_artist=True,
               boxprops=dict(facecolor=C_CYAN, alpha=0.7),
               medianprops=dict(color=C_BLUE, linewidth=2), manage_ticks=False)
    ax.set_xticks(soi_counts)
    ax.set_xticklabels(soi_counts)
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('Per-Worker Throughput (ops/sec)')
    ax.set_title('SoI Sweep — Victim Worker Distribution')
    ax.set_ylim(bottom=0)
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'per_worker_distribution.png'))

    totals = [victim_df[victim_df['soi_workers'] == s]['total_ops_sec'].sum() for s in soi_counts]
    cvs = []
    for s in soi_counts:
        vals = victim_df[victim_df['soi_workers'] == s]['total_ops_sec'].values
        mean = vals.mean()
        cvs.append(vals.std() / mean * 100 if mean > 0 else 0)

    fig, ax1 = plt.subplots(figsize=(8, 5))
    line_tp = ax1.plot(soi_counts, totals, '-o', color=C_BLUE, linewidth=2, markersize=6, label='Victim total throughput')
    ax1.set_xlabel('SoI Workers')
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
    ax1.set_title('SoI Sweep — Victim Throughput vs Fairness')
    ax1.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'per_worker_fairness.png'))


SOI_COLORS = {
    'cpu': C_BLUE, 'l1d': C_CYAN, 'l2': C_PURPLE, 'l3': C_GREEN,
    'membw': C_ORANGE, 'memcap': '#999999', 'iobw': C_RED, 'iops': '#8B4513',
}
SOI_MARKERS = {
    'cpu': 'o', 'l1d': 's', 'l2': '^', 'l3': 'D',
    'membw': 'v', 'memcap': 'x', 'iobw': 'P', 'iops': '*',
}


def plot_soi_comparison(soi_csvs: list):
    """Generate comparison plots across multiple SoI types."""
    if len(soi_csvs) < 2:
        return

    soi_csvs = sorted(soi_csvs, key=lambda p: pd.read_csv(p, nrows=1)['soi_type'].iloc[0])

    entries = []
    for csv_path in soi_csvs:
        df = pd.read_csv(csv_path)
        soi_type = df['soi_type'].iloc[0]
        if soi_type == 'ioops':
            soi_type = 'iops'
        entries.append((soi_type, df, csv_path))

    out_dir = str(Path(soi_csvs[0]).parent.parent)
    victim_workers = int(entries[0][1]['victim_workers'].iloc[0])

    print(f"  SoI comparison -> {out_dir}/")

    for i, (soi_type, df, csv_path) in enumerate(entries):
        if 'victim_cpu_change_pct' not in df.columns and 'victim_cpu_ops' in df.columns:
            base_cpu = df['victim_cpu_ops'].iloc[0]
            base_io = df['victim_io_ops'].iloc[0]
            df['victim_cpu_change_pct'] = ((df['victim_cpu_ops'] - base_cpu) / base_cpu * 100.0) if base_cpu > 0 else 0.0
            df['victim_io_change_pct'] = ((df['victim_io_ops'] - base_io) / base_io * 100.0) if base_io > 0 else 0.0
            entries[i] = (soi_type, df, csv_path)

    has_split_change = all('victim_cpu_change_pct' in df.columns and 'victim_io_change_pct' in df.columns
                           for _, df, _ in entries)
    is_mixed = has_split_change and all(
        df['victim_cpu_ops'].max() > 0 and df['victim_io_ops'].max() > 0
        for _, df, _ in entries
    )

    if is_mixed:
        fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 8), sharex=True, layout='constrained')
        for soi_type, df, _ in entries:
            color = SOI_COLORS.get(soi_type, 'gray')
            marker = SOI_MARKERS.get(soi_type, 'o')
            ax_cpu.plot(df['soi_workers'], df['victim_cpu_change_pct'], f'-{marker}',
                        color=color, linewidth=2, markersize=6, label=soi_type)
            ax_io.plot(df['soi_workers'], df['victim_io_change_pct'], f'-{marker}',
                       color=color, linewidth=2, markersize=6, label=soi_type)
        ax_cpu.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax_cpu.set_ylabel('CPU Throughput Change (%)')
        ax_cpu.legend(loc='best', ncol=2, fontsize=9)
        ax_cpu.grid(True, alpha=0.3)
        ax_io.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax_io.set_xlabel('SoI Workers')
        ax_io.set_ylabel('IO Throughput Change (%)')
        ax_io.legend(loc='best', ncol=2, fontsize=9)
        ax_io.grid(True, alpha=0.3)
        fig.suptitle(f'SoI Comparison — Victim Degradation ({victim_workers} victims)')
    else:
        fig, ax = plt.subplots(figsize=(10, 6))
        for soi_type, df, _ in entries:
            color = SOI_COLORS.get(soi_type, 'gray')
            marker = SOI_MARKERS.get(soi_type, 'o')
            ax.plot(df['soi_workers'], df['victim_change_pct'], f'-{marker}',
                    color=color, linewidth=2, markersize=6, label=soi_type)
        ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('Victim Throughput Change (%)')
        ax.set_title(f'SoI Comparison — Victim Degradation ({victim_workers} victims)')
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend(loc='best', ncol=2, fontsize=9)
        ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_degradation.png'))

    if is_mixed:
        fig, (ax_cpu, ax_io) = plt.subplots(1, 2, figsize=(14, 5), layout='constrained')
        for ax_bar, change_col, title_label in [
            (ax_cpu, 'victim_cpu_change_pct', 'CPU'),
            (ax_io, 'victim_io_change_pct', 'IO'),
        ]:
            types = []
            final_degs = []
            colors = []
            for soi_type, df, _ in entries:
                types.append(soi_type)
                final_degs.append(df[change_col].iloc[-1])
                colors.append(SOI_COLORS.get(soi_type, 'gray'))
            order = sorted(range(len(final_degs)), key=lambda i: final_degs[i])
            types = [types[i] for i in order]
            final_degs = [final_degs[i] for i in order]
            colors = [colors[i] for i in order]
            bars = ax_bar.bar(types, final_degs, color=colors, alpha=0.8, edgecolor='black', linewidth=0.5)
            ax_bar.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
            for bar, val in zip(bars, final_degs):
                ax_bar.text(bar.get_x() + bar.get_width() / 2, val - 1.5,
                            f'{val:.1f}%', ha='center', va='top', fontsize=9, fontweight='bold')
            ax_bar.set_xlabel('SoI Type')
            ax_bar.set_ylabel(f'{title_label} Throughput Change (%)')
            ax_bar.set_title(f'{title_label} Impact')
            ax_bar.grid(True, alpha=0.3, axis='y')
        max_soi = int(entries[0][1]['soi_workers'].max())
        fig.suptitle(f'SoI Impact — {max_soi} SoI Workers ({victim_workers} victims)')
    else:
        fig, ax = plt.subplots(figsize=(10, 5))
        types = []
        final_degs = []
        colors = []
        for soi_type, df, _ in entries:
            types.append(soi_type)
            final_degs.append(df['victim_change_pct'].iloc[-1])
            colors.append(SOI_COLORS.get(soi_type, 'gray'))
        order = sorted(range(len(final_degs)), key=lambda i: final_degs[i])
        types = [types[i] for i in order]
        final_degs = [final_degs[i] for i in order]
        colors = [colors[i] for i in order]

        bars = ax.bar(types, final_degs, color=colors, alpha=0.8, edgecolor='black', linewidth=0.5)
        ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        for bar, val in zip(bars, final_degs):
            ax.text(bar.get_x() + bar.get_width() / 2, val - 1.5,
                    f'{val:.1f}%', ha='center', va='top', fontsize=9, fontweight='bold')
        ax.set_xlabel('SoI Type')
        ax.set_ylabel('Victim Throughput Change (%)')
        max_soi = int(entries[0][1]['soi_workers'].max())
        ax.set_title(f'SoI Impact — {max_soi} SoI Workers ({victim_workers} victims)')
        ax.grid(True, alpha=0.3, axis='y')
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_bar.png'))

    fig, ax = plt.subplots(figsize=(10, 6))
    for soi_type, df, _ in entries:
        color = SOI_COLORS.get(soi_type, 'gray')
        marker = SOI_MARKERS.get(soi_type, 'o')
        baseline = df['victim_total_ops'].iloc[0]
        if baseline > 0:
            normalized = df['victim_total_ops'] / baseline * 100
            ax.plot(df['soi_workers'], normalized, f'-{marker}',
                    color=color, linewidth=2, markersize=6, label=soi_type)
    ax.axhline(y=100, color='gray', linestyle='--', alpha=0.5, label='Baseline')
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('Victim Throughput (% of baseline)')
    ax.set_title(f'SoI Comparison — Normalized Victim Throughput ({victim_workers} victims)')
    ax.set_xlim(left=0)
    ax.set_ylim(bottom=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.legend(loc='best', ncol=2, fontsize=9)
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_normalized.png'))
