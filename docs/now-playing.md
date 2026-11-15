# Now Playing widget

The track and artist that is playing, and whether it is paused. It shows the
same media session as Windows' own media controls, so it works with Spotify,
browsers, the Media Player app and other apps that report playback to Windows.

```
Running Up That Hill
Kate Bush · Paused
```

A paused or stopped player also gets a small pause badge on the icon. With
nothing playing, the widget says **Nothing playing**. Titles too long for the
pill scroll.

Off by default. Switch it on under **Widgets** in the dashboard. There is
nothing to set up.

## Use

- **Play/pause and next track:** the two buttons control the player Windows'
  own media controls show, through the same media session. If the player
  refuses, they send the keyboard's play/pause or next media key instead.
  There is no previous-track button, to save room.
- **Double-click:** **Open media player** brings up the app that is playing,
  through its Windows app id. Double-clicking a button does not count.
- **Right-click:** the usual placement and double-click options.

## Data

The session is read every two seconds through Windows'
`GlobalSystemMediaTransportControlsSessionManager`, on this PC only. Nothing is
sent anywhere.

Run `bettertaskbar-widgets.exe --check=now-playing | more` to see what Windows
reports.
