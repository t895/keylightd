#!/bin/bash

cargo build --release
sudo systemctl disable --now keylightd
sudo cp ./target/release/keylightd /usr/local/bin

sudo cp ./etc/keylightd.service /etc/systemd/system
sudo systemctl enable --now keylightd
