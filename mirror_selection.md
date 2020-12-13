# Mirror selection

Flexo attempts to choose a mirror that is suitable for you, i.e., a mirror with a sufficiently low latency
that is able to utilize your bandwidth.

This mirror selection mechanism is currently in the process of being improved, so if it does not work well for you,
I would appreciate it if you open an issue. In addition, you should try to modify your Flexo configuration:
The settings under the `[mirrors_auto]` section in your `/etc/flexo/flexo.toml` file determine how the mirrors are
selected, and which criteria the mirrors need to fulfill in order to be considered for selection. 

Try the following modifications in your `/etc/flexo/flexo.toml` file in that order:

1. If your configuration includes `ipv6 = true`, set it to `false` instead.
   This will exclude fewer mirrors from the selection process.
2. Modify the `allowed_countries` setting to include your own country and a few neighboring countries.
    Keep in mind that in some countries, no mirrors or very few mirrors are available, so in that case, make
    sure to include sufficiently many countries. Check out https://www.archlinux.org/mirrors/status/ to see
   how many mirrors are available in your country.
3. Modify the `max_score` setting: This score is just a very rough estimate of a mirror's performance,
   so it's possible that you're excluding too many sufficiently good mirrors if that setting is too low.
4. Modify the `timeout` setting: The default value should be fine for most users, but if you happen to have a high
   latency connection towards most mirrors, this setting should be increased.

Keep in mind that, after editing this file, you need to remove the previously cached latency test results and restart
Flexo before the changes take effect:
```bash
rm /var/cache/flexo/state/latency_test_results.json
systemctl restart flexo
```

