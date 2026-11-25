(function () {
  var mode = localStorage.getItem('theme-mode') || 'system'
  var isDark = mode === 'dark' || (
    mode === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches
  )
  var background = isDark ? 'rgb(16,16,20)' : 'rgb(242,242,247)'
  var foreground = isDark ? 'rgb(241,241,245)' : 'rgb(30,30,33)'
  document.documentElement.style.background = background
  document.documentElement.style.color = foreground
  document.documentElement.className = isDark ? 'dark-theme' : 'light-theme'
}())
