# Install

llevant runs on **Windows** and **Linux** (64-bit x86). There are no macOS builds
yet. Pick one of the three ways below; the first suits most people.

## With Python (recommended)

If you have Python 3.9 or later:

```sh
pip install llevant
```

This gives you all of llevant: the Python module, the `llevant` command and the
viewer. Check that it works:

```sh
python -m llevant --version      # llevant 0.1.0
python -m llevant view model.i   # the viewer, on your model
python -m llevant view           # the viewer's start screen
```

!!! tip "Use a virtual environment"

    To keep llevant apart from your other Python packages:

    === "Windows"

        ```bat
        python -m venv llevant-env
        llevant-env\Scripts\activate
        pip install llevant
        ```

    === "Linux"

        ```sh
        python3 -m venv llevant-env
        source llevant-env/bin/activate
        pip install llevant
        ```

    Run the `activate` line again in each new terminal before using llevant.

Everywhere these pages write `llevant ...`, you can type `python -m llevant ...`
instead. For data frames, also `pip install pandas` (or `polars`).

## Download the programs

Each [release on GitHub](https://github.com/Fusion4Energy/llevant/releases) has a zip
(Windows) and a tar.gz (Linux) with two programs, nothing to install:

- `llevant`: the command line;
- `llevant-viewer`: the viewer.

Unpack it anywhere, and put the folder on your `PATH` to run them from any terminal.
On Windows you can also drag a model file onto `llevant-viewer.exe`.

!!! note "Linux"

    The downloaded Linux programs run on any distribution with glibc 2.28 or later:
    RHEL, Rocky and Alma 8 and later, Ubuntu 20.04 and later, Debian 10 and later.
    The viewer also needs a desktop (X11 or Wayland) with GTK 3, which desktop
    installations have.

## Build from source

With [Rust](https://rustup.rs) 1.95 or later:

```sh
cargo install llevant-cli --features gpu,viewer   # `llevant`, with `render --gpu` and `llevant view`
cargo install llevant-viewer                      # `llevant-viewer`
```

On Windows, the GPU renderer and the viewer need Visual Studio 2022 17.11 or later (its
C++ build tools), and the build downloads Microsoft's DirectX shader compiler (DXC) from
GitHub once. Without a GPU driver (virtual machines, remote desktops), Windows offers only
its software adapter (WARP), which llevant does not use for now: `--gpu` and `gpu=True`
report an error, and the viewer traces on the CPU.

On Linux, the viewer needs some system libraries to build. On Debian or Ubuntu:

```sh
sudo apt-get install libxkbcommon-dev libwayland-dev libgtk-3-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libssl-dev pkg-config
```

Without `--features gpu,viewer` the build is much quicker, and has everything but the
GPU renderer and the viewer.

## What your computer needs

- **Memory.** Small and medium models need little. The largest models need several
  gigabytes: the 425 000-cell reference model takes about 4–5 GB the first time it is
  opened, and less afterwards.
- **A graphics card** makes the viewer's 3D view fast. Any card with Vulkan, DirectX
  12 or Metal drivers works. Without one, the viewer draws on the processor (slower,
  but exact).
- **Disk.** Opened models are kept in a cache of up to 8 GB, so that they open in a
  second the next time ([where it is](help.md#the-model-cache)).

Next: [Getting started](getting-started.md).
