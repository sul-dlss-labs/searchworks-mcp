{{/*
Expand the name of the chart.
*/}}
{{- define "searchworks-mcp.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Fully qualified app name. If the release name already contains the chart name
(as with an ArgoCD Application named searchworks-mcp), use it unchanged so the
Service is simply "searchworks-mcp".
*/}}
{{- define "searchworks-mcp.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "searchworks-mcp.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "searchworks-mcp.labels" -}}
helm.sh/chart: {{ include "searchworks-mcp.chart" . }}
{{ include "searchworks-mcp.selectorLabels" . }}
app.kubernetes.io/version: {{ .Values.image.tag | default .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "searchworks-mcp.selectorLabels" -}}
app.kubernetes.io/name: {{ include "searchworks-mcp.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Container port: the port half of config.bindAddress, so the two cannot drift.
*/}}
{{- define "searchworks-mcp.containerPort" -}}
{{- splitList ":" .Values.config.bindAddress | last -}}
{{- end }}

{{/*
MCP_ALLOWED_HOSTS: the Service name and loopback addresses, then
config.allowedHosts, comma-separated with duplicates removed.
*/}}
{{- define "searchworks-mcp.allowedHosts" -}}
{{- $hosts := list (include "searchworks-mcp.fullname" .) "localhost" "127.0.0.1" "::1" -}}
{{- $hosts = concat $hosts .Values.config.allowedHosts -}}
{{- $hosts | uniq | join "," -}}
{{- end }}
