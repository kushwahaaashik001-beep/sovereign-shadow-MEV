@echo on
set total=0
for /r %%f in (*.rs) do (
  echo "%%f" | findstr /i /c:"\target\" >nul || (
    if not "%%f"=="*.rs.bak" (
      for /f %%a in ('type "%%f" ^| find /c /v ""') do set /a total+=%%a
    )
  )
)
echo Total Rust code lines: %total%
