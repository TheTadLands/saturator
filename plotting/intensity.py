"""Plots for process-based intensity sweep experiments."""

import os
from pathlib import Path

import matplotlib.pyplot as plt
import pandas as pd

from .common import C_BLUE, C_CYAN, C_RED, format_number, save_fig


def plot_per_worker_intensity_sweep(csv_path: str):
    """Box plot of base workers + probe worker line for intensity sweep per-worker CSVs."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    intensities = sorted(df['probe_intensity'].unique())
    total_workers = int(df['workers'].iloc[0])
    probe_id = total_workers - 1

    base_df = df[df['worker_id'] != probe_id]
    probe_df = df[df['worker_id'] == probe_id]

    print(f"  -> {folder}/")

    fig, ax = plt.subplots(figsize=(10, 5))

    data = [base_df[base_df['probe_intensity'] == i]['total_ops_sec'].values for i in intensities]
    ax.boxplot(data, positions=intensities, widths=0.03, patch_artist=True,
               boxprops=dict(facecolor=C_CYAN, alpha=0.6),
               medianprops=dict(color=C_BLUE, linewidth=2),
               manage_ticks=False)

    probe_vals = []
    for i in intensities:
        rows = probe_df[probe_df['probe_intensity'] == i]['total_ops_sec'].values
        probe_vals.append(rows[0] if len(rows) > 0 else 0)
    ax.plot(intensities, probe_vals, '-o', color=C_RED, linewidth=2, markersize=6, label='Probe worker')

    ax.set_xlabel('Probe Intensity')
    ax.set_ylabel('Per-Worker Throughput (ops/sec)')
    ax.set_title(f'Intensity Sweep — Base Workers (box) vs Probe Worker')
    ax.set_ylim(bottom=0)
    ax.legend()
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'per_worker_intensity.png'))
