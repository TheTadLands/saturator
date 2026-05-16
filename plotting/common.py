"""Shared helpers and color palette for the plotting modules."""

import os
from pathlib import Path

import matplotlib.pyplot as plt


# Colorblind-friendly palette (Wong 2011)
C_BLUE = '#0072B2'
C_ORANGE = '#E69F00'
C_GREEN = '#009E73'
C_RED = '#D55E00'
C_CYAN = '#56B4E9'
C_PURPLE = '#CC79A7'


def format_number(x):
    """Format large numbers with K/M suffixes."""
    if x >= 1_000_000:
        return f'{x/1_000_000:.1f}M'
    elif x >= 1_000:
        return f'{x/1_000:.1f}K'
    else:
        return f'{x:.0f}'


def save_fig(fig, path):
    """Save figure to path with tight layout, close it, and print the filename."""
    if not fig.get_constrained_layout():
        fig.tight_layout(pad=0.5, h_pad=None, w_pad=None, rect=(0, 0.02, 1, 1))
    fig.savefig(path, dpi=150)
    plt.close(fig)
    print(f"    {Path(path).name}")


def detect_experiment(name):
    """Return (label, mode, mode_plural, x_label) from CSV stem name."""
    is_cpu = 'cpu' in name.lower()
    is_proc = 'proc' in name.lower() or 'worker' in name.lower()
    label = 'CPU' if is_cpu else 'I/O'
    mode = 'Process' if is_proc else 'Thread'
    mode_plural = 'Processes' if is_proc else 'Threads'
    x_label = 'Worker Count' if is_proc else 'Thread Count'
    return label, mode, mode_plural, x_label


PERF_COLS = ['l1d_load_misses', 'llc_load_misses', 'cache_misses',
             'instructions', 'cycles', 'ipc',
             'l1d_miss_per_kinsn', 'llc_miss_per_kinsn']


def has_perf(df):
    """Check if a DataFrame has hardware perf counter columns."""
    return 'ipc' in df.columns and 'l1d_load_misses' in df.columns


def plot_perf_metrics(df, x_col, x_label, title_prefix, folder, throughput_col=None):
    """Generate perf counter plots if the columns are present.

    Produces:
    - cache_misses.png: L1d, LLC, and total cache misses vs x_col
    - ipc_and_miss_rate.png: IPC (left axis) + miss rates per kilo-instruction (right axis)
    - throughput_vs_ipc.png: throughput (left) vs IPC (right), if throughput_col given
    """
    if not has_perf(df):
        return

    x = df[x_col]

    # 1. Cache misses (raw counts)
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(x, df['l1d_load_misses'], '-o', color=C_BLUE, linewidth=2,
            markersize=6, label='L1d load misses')
    ax.plot(x, df['llc_load_misses'], '-s', color=C_GREEN, linewidth=2,
            markersize=6, label='LLC load misses')
    ax.plot(x, df['cache_misses'], '-^', color=C_RED, linewidth=2,
            markersize=6, label='Cache misses (total)')
    ax.set_xlabel(x_label)
    ax.set_ylabel('Count (per measurement window)')
    ax.set_title(f'{title_prefix} — Cache Misses')
    ax.set_ylim(bottom=0)
    ax.legend(loc='best', fontsize=9)
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'cache_misses.png'))

    # 2. IPC + miss rates per kilo-instruction
    fig, ax1 = plt.subplots(figsize=(8, 5))
    line_ipc = ax1.plot(x, df['ipc'], '-o', color=C_BLUE, linewidth=2,
                        markersize=6, label='IPC')
    ax1.set_xlabel(x_label)
    ax1.set_ylabel('Instructions per Cycle', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.set_ylim(bottom=0)
    ax1.grid(True, alpha=0.3)

    ax2 = ax1.twinx()
    line_l1d = ax2.plot(x, df['l1d_miss_per_kinsn'], '-s', color=C_ORANGE,
                        linewidth=2, markersize=5, label='L1d misses/Kinsn')
    line_llc = ax2.plot(x, df['llc_miss_per_kinsn'], '-^', color=C_GREEN,
                        linewidth=2, markersize=5, label='LLC misses/Kinsn')
    ax2.set_ylabel('Misses per Kilo-Instruction')
    ax2.set_ylim(bottom=0)

    lines = line_ipc + line_l1d + line_llc
    ax1.legend(lines, [l.get_label() for l in lines], loc='best', fontsize=9)
    ax1.set_title(f'{title_prefix} — IPC & Miss Rates')
    save_fig(fig, os.path.join(folder, 'ipc_and_miss_rate.png'))

    # 3. Throughput vs IPC (dual axis)
    if throughput_col and throughput_col in df.columns:
        fig, ax1 = plt.subplots(figsize=(8, 5))
        line_tp = ax1.plot(x, df[throughput_col], '-o', color=C_BLUE,
                           linewidth=2, markersize=6, label='Throughput')
        ax1.set_xlabel(x_label)
        ax1.set_ylabel('Throughput (ops/sec)', color=C_BLUE)
        ax1.tick_params(axis='y', labelcolor=C_BLUE)
        ax1.set_ylim(bottom=0)
        ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

        ax2 = ax1.twinx()
        line_ipc = ax2.plot(x, df['ipc'], '-s', color=C_ORANGE, linewidth=2,
                            markersize=5, label='IPC')
        ax2.set_ylabel('Instructions per Cycle', color=C_ORANGE)
        ax2.tick_params(axis='y', labelcolor=C_ORANGE)
        ax2.set_ylim(bottom=0)

        lines = line_tp + line_ipc
        ax1.legend(lines, [l.get_label() for l in lines], loc='best')
        ax1.set_title(f'{title_prefix} — Throughput vs IPC')
        ax1.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'throughput_vs_ipc.png'))


def ext_subdir(folder, subdir):
    """Return `folder/subdir`, creating it if needed. Used to group ext-workload
    plots into timeseries/, per_sample/, utilization/, per_worker/ rather than
    letting ~20 PNGs pile up at the top level."""
    out = os.path.join(folder, subdir)
    os.makedirs(out, exist_ok=True)
    return out
