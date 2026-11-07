# Simple flow: download and build first, install later
**Idea:** A timer downloads and builds packages in background. Your update command then only installs them.
Words, in plain words:
- systemd = system that runs background tasks on a schedule
- sudo = giving admin rights for one command
- sudoers = file that controls who gets sudo
- base-devel = tools needed to build programs
- git = tool that downloads code from a hosted copy
- cache = folder where downloaded files are kept
- snapshot = a saved copy you can roll back to
- kernel = main core of the operating system
- kernel modules = extra code the kernel loads; DKMS = tool that rebuilds them when the kernel changes.
- RSS = a news feed format a reader app can poll
- libraries = shared code that programs use
- lock file = a file that means a job is running now
- manual intervention = news that tells you to do a step by hand
- NOPASSWD = permission to run one command without typing a password
- home-made repo = a package list you made yourself
- AUR = place where people post build recipes, not checked
- sandbox = separate build area. It does not make a bad recipe safe.
- free-space gate = check in script. Script stops before download if disk is too full.
- dry run = check what would be deleted. Nothing is deleted.
- inode = slot for a file name. You can run out of slots even if disk looks empty.
- vacuum = delete old logs to free space.
- mtree = list of file sizes and times saved when a package was installed.
Before automating, set up a snapshot tool so you can roll back if an install breaks things.
Limit the snapshot tool. No limit means snapshots eat your disk. See Step 1.
## Step 1: Set up once
Flags: -S = install, -y = refresh lists, -u = upgrade, -w = download only, -i = --info (show package info), -U = install a built package file, -Qua = list AUR updates only. Do not change these flags.
Step 1 is the one deliberate upgrade — read news first (Way #5). Check https://archlinux.org/news/ before you run the upgrade line below.
```bash
sudo usermod -aG wheel $USER
sudo pacman -S base-devel git pacman-contrib
git clone https://aur.archlinux.org/yay.git
cd yay
makepkg -si
yay -Syuu
yay -S aurto
aurto init
```
`pacman-contrib` provides `pacdiff`, `paccache`, and `paccache.timer`.
yay builds AUR recipes and installs them. aurto builds AUR recipes in background. makepkg builds one recipe.
Wheel group: adding yourself to `wheel` does nothing until `%wheel` is uncommented in the sudoers file and you log out and back in. If sudo already works, skip the usermod line.
`makepkg -si` for yay downloads Go tools by itself. Big download. It also asks for a sudo password and asks to install build deps. Type your password when asked. That prompt is expected.
What aurto init does by itself: it creates the repo. It adds an `Include = /etc/pacman.d/aurto` line to `/etc/pacman.conf`. The repo is defined in the included file.
Run it as `aurto init` with no sudo prefix. It asks for a sudo password inside. That prompt is expected, not a failure.
What you do by hand: edit `/etc/aurto/trusted-users` and add maintainer names. Then add AUR packages one by one.
Do not use `aurto add $(pacman -Qqm)` as one silent step. If no AUR packages are installed, it expands to zero args and fails. Each package may ask `Trust maintainer(s)? [y/N]`. Pressing Enter means N = not added. Run `aurto add <pkg>` for one package at a time and answer y only if you trust it.
Success looks like this: after init, the `Include = /etc/pacman.d/aurto` line appears in `pacman.conf`.
Enable linger so user tasks keep running after logout: `loginctl enable-linger $USER`
Clean download cache on a schedule: `sudo systemctl enable --now paccache.timer`
paccache.timer runs once a week by default. With daily download, change it to daily. Seven days pile up too fast.
Make a drop-in for daily clean:
```bash
sudo systemctl edit paccache.timer
```
Add this:
```ini
[Timer]
OnCalendar=
OnCalendar=daily
```
The empty `OnCalendar=` line clears the old weekly time. Without it, daily would be added to weekly, not replace it. Then reload: `sudo systemctl daemon-reload`
Make timer keep only 1 copy. Timer keeps 3 by default. On small disk keep 1. Run:
```bash
echo "PACCACHE_ARGS='-k1'" | sudo tee -a /etc/conf.d/pacman-contrib
```
`-a` means add the line, not overwrite the whole file. If the line is already there, open the file and check it instead of adding a second copy. Do not use tee without `-a`.
This makes timer match Step 4 manual clean. PACCACHE_ARGS = extra options for timer clean.
Point the timer at the `aur-prefetch` binary. `aur-prefetch fetch` runs only `sudo -n pacman -Syuw --noconfirm`. AUR builds are handled by aurto's own root update-aurto timers installed by aurto init.
The free-space gate lives in the binary, so the timer needs no wrapper. The block below is the teaching example: it shows the rule the binary implements. Do not install it as a second gate.
```bash
#!/bin/bash
# Free-space gate. Stop before pacman if disk is too full.
avail_kb=$(df --output=avail / | tail -1)
need_kb=2097152
if [ "$avail_kb" -lt "$need_kb" ]; then
  echo "ABORT: only $((avail_kb/1024)) MiB free on /" >&2
  exit 1
fi
sudo -n pacman -Syuw --noconfirm
```
2097152 KB is 2 GB. That is the floor. Rule: keep free space at least 2 times the download size plus a 2 to 5 GB floor. Use 5 GB floor if you use snapshots. The binary applies the same rule to the CacheDir filesystem and to `/`.
Note: this gate guards only `pacman -Syuw`. It does NOT guard aurto builds. aurto builds use same disk with no check. Builds can still fill disk even when gate passes. Check build folders in Step 4.
```bash
chmod +x ~/.local/bin/aur-prefetch.sh   # teaching example only
```
Add one sudoers line for that exact command only. The file is `/etc/sudoers.d/aur-prefetch`. Edit it with `sudo visudo -f /etc/sudoers.d/aur-prefetch`. Use NOPASSWD for `/usr/bin/pacman -Syuw --noconfirm`. No other commands. No wildcards.
Create two files. This is two files, not one. Each block below is one file. Do not paste both into one file.

File `~/.config/systemd/user/aur-prefetch.service`:
```ini
[Unit]
Description=Prefetch packages
[Service]
Type=oneshot
ExecStart=%h/.local/bin/aur-prefetch fetch
```

File `~/.config/systemd/user/aur-prefetch.timer`:
```ini
[Timer]
OnCalendar=daily
Persistent=true
[Install]
WantedBy=timers.target
```
The .service is the job (runs the command once). The .timer is the clock (runs it daily).
%h = your home folder. Type=oneshot = run once and stop. OnCalendar=daily = run once a day. Persistent=true = if the computer was off at run time, run soon after you turn it on. WantedBy = start with timers.
```bash
systemctl --user daemon-reload
systemctl --user enable --now aur-prefetch.timer
systemctl --user list-timers
```
daemon-reload = tell systemd to read new files. list-timers = list timers and next run time.
Expected output: a line names `aur-prefetch.timer` and shows a future time.
Check logs: `journalctl --user -u aur-prefetch.service`. journalctl = show logs.
Cap logs. Daily timers make many logs. No cap means logs eat disk.
Edit `/etc/systemd/journald.conf`. Storage means where logs are kept. Persistent means keep them on disk. Without this, SystemMaxUse does nothing. Logs stay in RAM only.
First make the journal folder, then edit the file. Set these lines:
```ini
Storage=persistent
SystemMaxUse=500M
RuntimeMaxUse=100M
```
Then run:
```bash
sudo mkdir -p /var/log/journal
sudo systemctl restart systemd-journald
```
Check log size any time: `journalctl --disk-usage`
Delete old logs now: `sudo journalctl --vacuum-size=500M --vacuum-time=7d`
Limit snapshots. If you use snapper on btrfs, edit `/etc/snapper/configs/root`. SPACE_LIMIT and FREE_LIMIT work only on btrfs. On other file systems they do nothing. Set these:
```ini
TIMELINE_CREATE="yes"
NUMBER_CLEANUP="yes"
TIMELINE_CLEANUP="yes"
NUMBER_LIMIT="10"
NUMBER_LIMIT_IMPORTANT="5"
TIMELINE_MIN_AGE="1800"
TIMELINE_LIMIT_HOURLY="5"
TIMELINE_LIMIT_DAILY="2"
TIMELINE_LIMIT_MONTHLY="1"
TIMELINE_LIMIT_YEARLY="0"
TIMELINE_LIMIT_WEEKLY="0"
TIMELINE_LIMIT_QUARTERLY="0"
SPACE_LIMIT="0.5"
FREE_LIMIT="0.2"
```
Number limit 10 plus important 5 means up to 15 kept. Normal and important count separate. Not 10 timeline only. Timeline limits above cap by age. Space limit and free limit stop snapshots when disk is tight. Tighten free limit on small disk, for example 0.3. No cleanup means disk fills.
Space and free limits need quota setup. Run once: `sudo snapper -c root setup-quota`. Limits must be ranges (like `10-20`) for space cleanup to work right. With single numbers, snapper may ignore space limits.
Check timers run: `systemctl list-timers | grep snapper`. Timers `snapper-timeline.timer` and `snapper-cleanup.timer` must be active or no cleanup happens.
List snapshots: `sudo snapper -c root list`
Delete old ones when disk is tight: `sudo snapper -c root delete <number>` List first, then delete by number.
Small root only: you can move download cache to /home. Advanced only. Skip this if you want simple setup. Skip it if home is 0700. Pacman 7 sandbox breaks with closed home. If you try it, do this first:
```bash
mkdir -p /home/user/.cache/pacman/pkg
sudo chown root:root /home/user/.cache/pacman/pkg
sudo chmod o+x /home/user /home/user/.cache /home/user/.cache/pacman /home/user/.cache/pacman/pkg
```
Then edit `/etc/pacman.conf`. Set:
```ini
CacheDir = /home/user/.cache/pacman/pkg/
ParallelDownloads = 5
```
Default is `/var/cache/pacman/pkg/`. Home is often bigger than root. ParallelDownloads 5 means 5 files at once. Lower number means less disk used at once.
Check it works: `pacman-conf CacheDir`, then test download: `sudo pacman -Sw tiny-package`. If test fails, remove the line or add DisableSandbox setting in pacman.conf. See man pacman.conf for DisableSandbox.
Know limits: aurto repo and build chroots stay on `/` even if downloads move. They do not move with CacheDir.
If you use btrfs, add compress. It saves space on cache and snapshots. Mount option is `compress=zstd`.
## Step 2: Background, automatic
Timer runs once a day. Arch part runs `pacman -Syuw`. That is refresh lists, upgrade, download only. It installs nothing.
Waiting for download is safe. But lists are new and system is old. That is not broken yet. You still need Step 3 soon. See Ways to mess up #1.
Prefetch changes some things only. It does not change other things.

| Prefetch changes | Prefetch does NOT change |
|---|---|
| Database lists in `/var/lib/pacman/sync` | Installed files in `/usr` |
| Download files in cache | Running kernel and libraries |
| Built packages in aurto repo | Config files in `/etc` |
| Log entry for download | Services that are running |

So prefetch only downloads and builds. You still need Step 3 install. See Ways to mess up #3.
AUR part: aurto's own root update-aurto timers build in sandbox. They put built packages in local repo. They use repo-add to make package list from folder. repo-add = tool that makes package list from a folder of built packages. aurto does this itself. Do not run repo-add by hand on the aurto repo. Repo is already in `pacman.conf` from init. So yay can find them.
Know the caches. Downloads, builds, and local repo are different folders. paccache cleans each CacheDir in pacman.conf. It does not clean builds or local repo.
- Cache 1: `/var/cache/pacman/pkg`. This is downloads. Every daily download adds a full copy here. If you set CacheDir to /home, use that path. Check with `pacman-conf CacheDir`.
- Cache 2: `~/.cache/yay/<pkg>/`. This is builds. Every AUR build leaves source, tarballs, and built files here. Old builds stay.
- Cache 3: `/var/cache/pacman/aurto`. This is local repo. Every aurto build adds a new .pkg.tar.zst file. Old files stay. aurto maintains its repo internally. Do not run repo-add by hand. Manual paccache on the repo dir can desync the database file and the files — the database says a file is there when it is gone. Leave cleanup to aurto unless you verified your version does it. Check `man aurto` for your version.
- Cache 4: build chroots `/var/lib/aurbuild` and `/var/lib/archbuild`. aurto and devtools keep a full build system here. Each is over 1 GB. They stay on `/`. Free-space gate does not guard them.
Check all: find your download path with `pacman-conf CacheDir` first, then:
```bash
du -sh /var/cache/pacman/pkg /var/cache/pacman/aurto ~/.cache/yay
du -sh /var/lib/aurbuild /var/lib/archbuild
```
If you moved CacheDir to /home, replace first path with /home path.
How big can a download be? Kernel is about 100 to 150 MB. Browser or desktop upgrade is often 300 MB to 1.5 GB. One big upgrade can be larger than free space on a small root like 16 GB or 20 GB.
Daily download grows cache 1 faster than manual updates. If you skip Step 3 for days, new versions pile up. Old copies stay until cleaner runs.
Estimate before you commit: `pacman -Qu` lists what will upgrade. `pacman -Si <pkg>` shows Download Size for one package.
## Step 3: Your turn
Check both disk space and file slots first. Run both:
```bash
df -h /
df -i /
```
`df -h` shows free disk. `df -i` shows free file slots. Full slots break install even if disk looks free.
Check lock file `/var/lib/pacman/db.lck` first. If it exists, check the service or `pgrep pacman`. If pacman is still running, wait.
Check Arch news. News is at https://archlinux.org/news/ Use the gated sequence. Do not install first. Manual steps can be before install or after install. Installing first can leave you unbootable. See Ways to mess up #5.
1. List upgrades: `yay -Qua` and `pacman -Qu`
2. Find last install time: check `/var/log/pacman.log` for last upgrade
3. Read news since that last install time
4. Match news names to packages in your upgrade list
5. Do pre-steps from news, if any
6. Install: run `yay -Syuu` with no sudo prefix
7. Do post-steps from news, if any
You can also read news by RSS feed. Add the Arch news RSS link to a feed reader or browser. That way you see new posts fast.
List updates: `yay -Qua`. Then run with no sudo prefix: `yay -Syuu`
Usually no building happens now. If a package builds now, aurto did not run today. Stop. Read its PKGBUILD first. That is the build recipe script.
Timing rule: install at your next session. Why install soon: you run programs built against old libraries while package lists point at new ones. That drift is not broken yet, but it gets risky. After refresh, (i) lists new + nothing installed = not broken yet, still needs Step 3 soon. (ii) anything installed from new lists = partial upgrade. If a day passes, run full `yay -Syuu` again. It re-checks everything. Never install single packages in the window. See Ways to mess up #1.
Disk full has two kinds. They need different fixes.
- Full during download: this is mostly safe. Partial files are re-downloaded. Clean cache, then retry. But note: `-Syuw` refreshes lists first. Even failed download leaves new lists with old system. You must still finish with full `yay -Syuu`. Do not install single packages between.
- Full during install: this is corruption. Package files are cut off mid-write. Database may claim a package is installed when it is not. Retry alone does not fix it.
Find broken packages: `pacman -Qk <pkg>` checks only that files are there. A file cut short but still there passes. Only `pacman -Qkk <pkg>` checks size and time from the mtree file. For corruption, rely on -Qkk, not -Qk.
If disk was full during install, do this repair after you free space:
```bash
sudo pacman -S <pkg>
```
Plain install overwrites bad files. Use `pacman -R <pkg>` first only if you must remove. Do not use `-Rs`. It removes dependencies too. That spreads harm.
If kernel was in the broken set, also run: `sudo mkinitcpio -P` This rebuilds boot images.
Then check kernel: `uname -r && ls /usr/lib/modules/`
If something fails:
- Disk low but install never started: clean caches in Step 4, then retry.
- Disk low during install: clean caches, then repair above for each broken package.
- Lock stuck: check `systemctl --user status aur-prefetch.service` or `pgrep pacman`. Timer status shows active waiting. It does not show if download is running. Only remove lock file if no pacman is running.
- Breaking news: do not install yet. Read news first.
- Build did not run today: rebuild by hand after reading PKGBUILD.
Finish Step 3 with pacdiff. Pacdiff is part of install, not extra tidy. Run it before reboot or service restart. See Ways to mess up #4.
## Step 4: After
Kernel check: run `uname -r`. Run `ls /usr/lib/modules/`. First command shows kernel running now. Second shows kernels on disk. Also run `pacman -Q linux`. That shows installed kernel version.
There are two cases.
(a) Reboot before install: new kernel is only in cache. It is not on disk under `/usr/lib/modules/`. Reboot boots old kernel again. That is safe. No mismatch. You still need Step 3.
(b) Install done but not rebooted: real mismatch. Install removed the running kernel's modules from `/usr/lib/modules/`. Running kernel is old, modules on disk are new. Programs may not load.
Reboot rule: reboot after Step 3 when kernel or libraries were upgraded, or news says so. See Ways to mess up #2.
Config check: run `sudo pacdiff`. It shows changed config files. `.pacnew` is new default config. `.pacsave` is your old file, saved when package was removed. Pacman never overwrites a config you changed. That is why pacdiff matters. See Ways to mess up #4.
Clean up. Do this after a good install. Clean each cache by itself.
Cache 1 downloads. Check your path first: `pacman-conf CacheDir`. Default is `/var/cache/pacman/pkg`. Use that path in all du checks. `paccache -d` is dry run. It shows space you can free. `paccache -r` keeps 3 newest versions by default. That means 3 kernels parked at about 450 MB. For tight disk use keep 1: `paccache -rk1`. For no keep on uninstalled only use `paccache -ruk0`. It deletes all cached copies of uninstalled packages only. Installed packages files stay. Full wipe is `pacman -Scc` or `paccache -rk0` without `-u`. Full wipe means re-download next time.
```bash
pacman-conf CacheDir
du -sh /var/cache/pacman/pkg
paccache -d
paccache -rk 1
```
If you moved CacheDir to /home, use /home path. paccache cleans each CacheDir in pacman.conf. Check both if you set two.
`pacman -Sc` removes old and not installed. `pacman -Scc` removes all. Use `-Scc` only if you accept full re-download next time.
Cache 2 yay builds. Check size:
```bash
du -sh ~/.cache/yay
du -sh ~/.cache/yay/* | sort -h | tail -20
```
Find old built files: `find ~/.cache/yay -name "*.pkg.tar.zst" -type f -mtime +30`
Delete after good install: `rm -rf ~/.cache/yay/<pkgname>` This removes source, tarballs, and built files. Installed package is untouched. Do this only after `pacman -Q <pkg>` shows good install.
Small files in yay cache are safe to delete. `~/.cache/yay/vcs.json` tracks git builds. Delete it and yay finds versions again next run. Completion cache files also rebuild next run. No install is lost.
Cache 3 aurto repo. Check size and check the database: `pacman -Sl aurto`. That lists what pacman thinks is in the repo. It should match the files on disk. Do not prune the repo dir by file date. Deleting by time can delete the current version. Do not run paccache on the repo dir. Leave cleanup to aurto unless you verified your version does it (check `man aurto`).
```bash
du -sh /var/cache/pacman/aurto
pacman -Sl aurto
```
Build chroots. Check size: `du -sh /var/lib/aurbuild /var/lib/archbuild`. Each over 1 GB. aurto uses first. devtools uses second. Chroots stay on `/`. Delete only when no build runs. Check: `pgrep -a aur-chroot` and `pgrep -a extra-x86_64-build`. Then delete if needed: `sudo rm -rf /var/lib/aurbuild /var/lib/archbuild`.
Logs and snapshots. Check log size: `journalctl --disk-usage`. Shrink: `sudo journalctl --vacuum-size=500M --vacuum-time=7d`.
Check snapshots: `sudo snapper -c root list`. Delete old ones if needed.
Build leftovers. In a built folder, `makepkg -c` removes src and intermediate files. It keeps PKGBUILD and final package.
Check temp folders: `du -sh /var/tmp /tmp`. List orphans: `ls /var/tmp/`. Interrupted builds leave `extra-*` folders there. Check size: `find /var/tmp -maxdepth 1 -name "extra-*" -exec du -sh {} +`. Confirm no build is running: `pgrep -a extra-x86_64-build`. Then delete the folder.
Run now-clean: `sudo systemd-tmpfiles --clean --prefix=/var/tmp`. It deletes only files older than age rules. It does not delete all.
## Ways to mess up
### 1. Installing anything between download and install — partial upgrade
Cause: Arch updates all the time, so packages must match. Programs need matching libraries. If you install even one package from new lists, that package is new but its libraries are old. The package may fail to start, or break other programs.
Two states. (i) Lists are new, nothing installed = not broken yet, still needs Step 3 soon. (ii) Anything installed from new lists = partial upgrade.
Worked example: you run `pacman -S firefox` after prefetch. Pacman pulls new firefox. New firefox needs new glibc. Old glibc is still installed as dependency. Firefox does not start. That is a partial upgrade.
Prevention: in the download to install window, do not run any of these:
- single `pacman -S <pkg>`
- `yay -S <aur-pkg>`
- graphical (GUI) updater like Pamac or Arch-Update
- `pacman -U` a cached `*.pkg.tar.zst` file
Do only Step 3. Step 3 is full `yay -Syuu`.
Detection:
- Check `/var/log/pacman.log`. Look for installs after the last download run. Example: `grep "installed\|upgraded" /var/log/pacman.log`
- Run `pacman -Qu`. If it is not empty, updates are still pending.
- Check files: `sudo pacman -Qkk <pkg>` for the one you installed. -Qk only checks presence. -Qkk checks size and time.
Recovery if you already did it:
1. Stop installing singles.
2. Check news since last install. See #5.
3. Run full `yay -Syuu`.
4. Run `sudo pacdiff`. See #4.
5. Reboot if kernel or libraries changed.
6. Check with `sudo pacman -Qkk <pkg>`.
7. If system is still broken, roll back snapshot from Step 1.
### 2. Rebooting before install — kernel mismatch
Cause: there are two different cases. Do not mix them.
(a) Reboot before install: new kernel is only a file in cache. It is not installed under `/usr/lib/modules/`. Reboot boots the old kernel again. You have not updated anything.
(b) Install done but not yet rebooted: real mismatch. Install removed the running kernel's module files. Running kernel is old. Modules on disk are new. Programs that need kernel modules may not load.
Prevention: reboot rule. Reboot after Step 3 when kernel or libraries were upgraded, or when news says so. Reboot does not finish an update. Install first, then reboot.
Detection: run all three:
```bash
uname -r
ls /usr/lib/modules/
pacman -Q linux
```
If first line does not match a folder in second list, you have case (b). If `pacman -Q linux` matches running kernel, you have case (a) or you are in sync.
Recovery:
(a) Rebooted before install: no repair needed. Just proceed to Step 3.
(b) Install done, not rebooted: reboot promptly. If programs still fail to load, reinstall linux plus your DKMS modules, run `sudo mkinitcpio -P` to rebuild boot images, then reboot again. If still broken, roll back snapshot from Step 1.
### 3. Thinking prefetch = updated
Cause: prefetch downloads only. It does not install. Many people think "timer ran, so I am up to date." That is wrong. Installed files, running kernel, configs, and services did not change.
Prevention: timer success does not mean system is current. You still need Step 3 install at your next session. Check pending updates before you call the system updated.
Detection: check pending updates:
```bash
pacman -Qu
yay -Qua
```
If either list is not empty, updates are pending. Check one package: `pacman -Q <pkg>` shows installed version. `pacman -Si <pkg>` shows available version. If versions differ, you are not updated. Check `/var/log/pacman.log`. Compare last download time to last install time. If cache grows, check `paccache -d` and `du -sh /var/cache/pacman/pkg`. Growing cache means downloads pile up.
Recovery if you believed this for days:
1. Run full `yay -Syuu`. Do not install singles. Cached downloads may be old. Newer versions may have replaced them.
2. Re-check news since last install, not since last download. Check log for last install time. See #5.
3. Run `sudo pacdiff`. See #4.
4. Reboot if kernel was in the set.
This is the same recovery as #1. See #1.
### 4. Skipping pacdiff — old configs
Cause: pacman never overwrites a config file you changed. On upgrade, new default lands as `.pacnew`. On removal, your old file is saved as `.pacsave`. So new defaults never apply unless you merge by hand.
Harm example: new version changes how a service starts, say a new timeout or a new start option. You keep the old config. New setting is ignored. Service still uses old value, or fails to start on next reboot. You only find out at reboot.
Prevention: pacdiff is a required gate every upgrade. It is part of finishing Step 3, not optional tidy. Run `sudo pacdiff` before reboot and before restarting the affected service.
Detection: if you skipped it for months, check backlog:
```bash
pacdiff -o
find /etc -name '*.pacnew' -o -name '*.pacsave'
```
`pacdiff -o` lists files without asking to merge. Sort by age to see the oldest first. Many files means a big backlog.
Recovery:
1. Back up `/etc` first. Example: `sudo cp -a /etc /root/etc-backup-$(date +%F)`
2. For each file, compare: `diff -u /etc/<file> /etc/<file>.pacnew`
3. Merge by hand. Keep your changes, add new defaults.
4. Delete the `.pacnew` file after merge.
5. For `.pacsave`, the package is removed. Review if you still need the file. Delete or save elsewhere.
6. Restart the service to test. Example: `sudo systemctl restart <service>`
7. Check news too. Config migrations are often announced there. See #5.
### 5. Skipping news — breaking change
Cause: automation cannot do manual intervention. Prefetch can download and build fine. It cannot do steps by hand. Install is the point of no return. Some manual steps must happen before install. Some must happen after. If you install first, you can leave the system unbootable.
Prevention: use the gated sequence in Step 3. List upgrades first. Find news since last install. Cross-reference package names in news to your upgrade list. Do pre-steps. Install. Do post-steps. Prefetch download is fine. Install is gated by news.
You can also subscribe to Arch news RSS feed. That helps you see breaking news fast.
Detection: if you suspect you missed news:
- Compare dates. Check last install time in `/var/log/pacman.log`. Compare to news archive dates.
- Run `systemctl --failed` for system services and `systemctl --user --failed` for user timers. Failed services can mean missed news.
- A failed user timer may just be the free-space gate stopping the run. That shows as failed but is not missed news. Check the log to see why it failed.
- Check pacdiff backlog. See #4. A backlog can mean a missed config migration.
- Check pacman warnings in `/var/log/pacman.log`.
Recovery if you already installed:
1. Read the news entry late. Do the manual step now.
2. Restart service or reboot if news says so.
3. Reinstall or repair the affected package. Example: `sudo pacman -S <pkg>`
4. If system is broken and does not boot, roll back snapshot from Step 1.
Fix forward vs roll back: fix forward if system boots and only one service or program is broken. Roll back if system is unbootable or many services are broken.
## Other mistakes
- Running builds as root. Never sudo yay or makepkg. Only timer gets sudo for exact prefetch command. No NOPASSWD ALL.
- Accepting a build without reading. PKGBUILD changes on each update. Read new PKGBUILD before you let a build run.
- Deleting the lock file while pacman still running corrupts the database. Check service/pgrep first.
- Thinking paccache cleans all. It cleans each CacheDir. yay builds, aurto repo, and chroots need own checks.
- Thinking disk full is always safe to retry. Full at download is mostly safe but still needs full install. Full at install is corruption. Reinstall broken packages.
- Forgetting aurto repo and chroots stay on `/`. Moving downloads to /home does not move them.
- Leaving snapshots, logs, and temp folders with no limit. They fill a small root slowly. Cap them in Step 1.
