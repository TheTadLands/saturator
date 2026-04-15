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


def ext_subdir(folder, subdir):
    """Return `folder/subdir`, creating it if needed. Used to group ext-workload
    plots into timeseries/, per_sample/, utilization/, per_worker/ rather than
    letting ~20 PNGs pile up at the top level."""
    out = os.path.join(folder, subdir)
    os.makedirs(out, exist_ok=True)
    return out
