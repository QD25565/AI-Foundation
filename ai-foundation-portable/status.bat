@echo off
if "%AI_ID%"=="" set AI_ID=my-ai
echo AI-Foundation starting as %AI_ID%
bin\notebook-cli.exe stats
