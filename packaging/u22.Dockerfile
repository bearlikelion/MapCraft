# Ubuntu 22.04 keeps the glibc the AppImage and the Godot Linux library need low; see linux-container.sh.
FROM ubuntu:22.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
	build-essential ca-certificates ccache curl file git ninja-build pkg-config python3 zip \
	libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev \
	&& rm -rf /var/lib/apt/lists/*
# The Godot extension needs CMake 3.25 or newer, and 22.04 ships 3.22.
RUN curl -sSL https://github.com/Kitware/CMake/releases/download/v3.31.6/cmake-3.31.6-linux-x86_64.tar.gz | tar -xz --strip-components=1 -C /usr/local
