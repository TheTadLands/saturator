#!/usr/bin/env python3
"""
Generate graphs from saturator CSV output files.

Usage:
    python plot_results.py                    # Plot all CSVs in ./output
    python plot_results.py output/*.csv       # Plot specific files
"""

import sys
import os
import glob
import pandas as pd
import matplotlib.pyplot as plt
from pathlib import Path


def plot_saturation(csv_path: str, output_dir: str):
    """Plot saturation curve (throughput vs thread/worker count)."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem

    # Determine if CPU or I/O saturation
    is_cpu = 'cpu' in name.lower()
    is_proc = 'proc' in name.lower() or 'worker' in name.lower()
    color = 'tab:blue' if is_cpu else 'tab:green'
    label = 'CPU' if is_cpu else 'I/O'
    mode = 'Process' if is_proc else 'Thread'
    x_label = 'Worker Count' if is_proc else 'Thread Count'

    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(14, 5))

    # Left plot: Total throughput
    ax1.plot(df['threads'], df['throughput_ops_sec'], f'-o', color=color, linewidth=2, markersize=6)

    # Mark the peak
    peak_idx = df['throughput_ops_sec'].idxmax()
    peak_threads = df.loc[peak_idx, 'threads']
    peak_throughput = df.loc[peak_idx, 'throughput_ops_sec']
    ax1.axvline(x=peak_threads, color='r', linestyle='--', alpha=0.7, label=f'Peak: {peak_threads} {mode.lower()}s')
    ax1.scatter([peak_threads], [peak_throughput], color='r', s=100, zorder=5)

    ax1.set_xlabel(x_label, fontsize=12)
    ax1.set_ylabel('Total Throughput (ops/sec)', fontsize=12)
    ax1.set_title(f'{label} Saturation ({mode}s) - Total Throughput', fontsize=14)
    ax1.legend(loc='upper center', bbox_to_anchor=(0.5, -0.12), ncol=2)
    ax1.grid(True, alpha=0.3)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda x, p: format_number(x)))

    # Right plot: Throughput per thread/worker
    ax2.plot(df['threads'], df['throughput_per_thread'], f'-s', color=color, linewidth=2, markersize=6)

    ax2.set_xlabel(x_label, fontsize=12)
    ax2.set_ylabel(f'Throughput per {mode} (ops/sec)', fontsize=12)
    ax2.set_title(f'{label} Saturation ({mode}s) - Per {mode} Efficiency', fontsize=14)
    ax2.grid(True, alpha=0.3)
    ax2.yaxis.set_major_formatter(plt.FuncFormatter(lambda x, p: format_number(x)))
    
    # Add annotation for efficiency drop
    first_throughput = df.loc[0, 'throughput_per_thread']
    last_throughput = df.loc[len(df)-1, 'throughput_per_thread']
    efficiency_drop = (1 - last_throughput / first_throughput) * 100
    ax2.annotate(f'{efficiency_drop:.0f}% efficiency loss\nat max threads', 
                 xy=(df['threads'].iloc[-1], last_throughput),
                 xytext=(df['threads'].iloc[-1] * 0.7, (first_throughput + last_throughput) / 2),
                 arrowprops=dict(arrowstyle='->', color='gray'),
                 fontsize=10, color='gray')
    
    output_path = os.path.join(output_dir, f'{name}.png')
    plt.tight_layout()
    plt.subplots_adjust(bottom=0.18)
    plt.savefig(output_path, dpi=150)
    plt.close()
    print(f"  Saved: {output_path}")


def plot_slack(csv_path: str, output_dir: str):
    """Plot slack analysis (baseline change vs extra threads)."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    
    # Determine if this is CPU or I/O baseline from filename
    # New format: slack_4cpu_adding_100pct_io.csv or slack_4io_adding_0pct_io.csv
    is_cpu_baseline = 'cpu_adding' in name or name.startswith('cpu_slack')
    baseline_type = 'CPU' if is_cpu_baseline else 'I/O'
    primary_col = 'cpu_ops' if is_cpu_baseline else 'io_ops'
    
    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(14, 5))
    
    # Left plot: Ops breakdown with dual y-axes
    color_cpu = 'tab:blue'
    color_io = 'tab:green'
    
    ax1.set_xlabel('Extra Threads', fontsize=12)
    ax1.set_ylabel('CPU ops/sec', fontsize=12, color=color_cpu)
    line1, = ax1.plot(df['extra_threads'], df['cpu_ops'], 'o-', color=color_cpu, 
                      label='CPU ops/s', linewidth=2, markersize=5)
    ax1.tick_params(axis='y', labelcolor=color_cpu)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda x, p: format_number(x)))
    
    # Secondary y-axis for I/O ops
    ax1_io = ax1.twinx()
    ax1_io.set_ylabel('I/O ops/sec', fontsize=12, color=color_io)
    line2, = ax1_io.plot(df['extra_threads'], df['io_ops'], 's-', color=color_io, 
                         label='I/O ops/s', linewidth=2, markersize=5)
    ax1_io.tick_params(axis='y', labelcolor=color_io)
    ax1_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda x, p: format_number(x)))
    
    # Combined legend below the plot
    ax1.legend(handles=[line1, line2], loc='upper center', bbox_to_anchor=(0.5, -0.12), ncol=2)
    ax1.set_title(f'{name.replace("_", " ").title()} - Throughput', fontsize=12)
    ax1.grid(True, alpha=0.3)
    
    # Right plot: Baseline change %
    ax2.plot(df['extra_threads'], df['baseline_change_pct'], 'r-o', linewidth=2, markersize=5)
    ax2.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    ax2.axhline(y=-20, color='orange', linestyle='--', alpha=0.7, label='-20% threshold')
    
    # Fill regions
    ax2.fill_between(df['extra_threads'], df['baseline_change_pct'], 0, 
                     where=(df['baseline_change_pct'] >= 0), alpha=0.3, color='green')
    ax2.fill_between(df['extra_threads'], df['baseline_change_pct'], 0, 
                     where=(df['baseline_change_pct'] < 0), alpha=0.3, color='red')
    
    ax2.set_xlabel('Extra Threads', fontsize=12)
    ax2.set_ylabel(f'{baseline_type} Baseline Change (%)', fontsize=12)
    ax2.set_title(f'{baseline_type} Baseline Degradation', fontsize=12)
    ax2.legend(loc='upper center', bbox_to_anchor=(0.5, -0.12), ncol=2)
    ax2.grid(True, alpha=0.3)
    
    output_path = os.path.join(output_dir, f'{name}.png')
    plt.tight_layout()
    plt.subplots_adjust(bottom=0.18)
    plt.savefig(output_path, dpi=150)
    plt.close()
    print(f"  Saved: {output_path}")


def format_number(x):
    """Format large numbers with K/M suffixes."""
    if x >= 1_000_000:
        return f'{x/1_000_000:.1f}M'
    elif x >= 1_000:
        return f'{x/1_000:.0f}K'
    else:
        return f'{x:.0f}'


def main():
    # Determine input files
    if len(sys.argv) > 1:
        csv_files = sys.argv[1:]
    else:
        csv_files = glob.glob('output/*.csv')
    
    if not csv_files:
        print("No CSV files found. Run saturator experiments first.")
        print("Usage: python plot_results.py [csv_files...]")
        sys.exit(1)
    
    # Ensure output directory exists
    output_dir = 'output'
    os.makedirs(output_dir, exist_ok=True)
    
    print(f"Plotting {len(csv_files)} file(s)...")
    
    for csv_path in sorted(csv_files):
        name = Path(csv_path).stem
        print(f"Processing: {name}")
        
        try:
            if 'throughput_vs_threads' in name or 'throughput_vs_workers' in name:
                plot_saturation(csv_path, output_dir)
            elif 'slack' in name:
                plot_slack(csv_path, output_dir)
            else:
                print(f"  Skipping unknown format: {name}")
        except Exception as e:
            print(f"  Error: {e}")
    
    print("\nDone!")


if __name__ == '__main__':
    main()
