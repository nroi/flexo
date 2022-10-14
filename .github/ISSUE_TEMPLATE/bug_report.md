---
name: Bug report
about: Create a report to help us improve
title: "[Potential Bug]"
labels: ''
assignees: nroi

---

**Describe the bug**
A clear and concise description of what the bug is.

**Installation method**
Have you installed Flexo from AUR, or are you running Flexo on Docker?

**Distribution**
Are you using the default Arch Linux on x86, Arch Linux ARM, Manjaro, or something else?

**Version**
If you've installed Flexo from AUR, provide the output of: `pacman -Qi flexo-git`. If you use Docker, provide the version tag of the Docker image you're using (check `docker image ls nroi/flexo` for example).

**Log**
If possible, provide the log output from the timeframe when the error has occurred. If you've installed Flexo from AUR, use journalctl, e.g.
```
journalctl --unit=flexo --since=today
```
If the error is reproducible, you may (optionally) also change the increase the log level to DEBUG. To do so, edit the file  `/usr/lib/systemd/system/flexo.service` and replace the following line:
```
Environment="RUST_LOG=info"
```
with:

```
Environment="RUST_LOG=debug"
```
Then, restart flexo:
`systemctl restart flexo`

journalctl should now provide more output that might help troubleshooting the issue

If you use, docker, use `docker logs` command.
