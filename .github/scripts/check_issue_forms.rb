require 'json'
require 'yaml'

# 校验标签清单与 Issue 表单：自动分流依赖这里约束的结构，改模板前先跑本脚本
root = File.expand_path('../..', __dir__)
labels = JSON.parse(File.read(File.join(root, '.github/labels.json')))
names = labels.map { |label| label.fetch('name') }
abort 'Label names must be unique' unless names.map(&:downcase).uniq.size == names.size
labels.each do |label|
  abort "Invalid label color: #{label['name']}" unless /\A[0-9a-fA-F]{6}\z/.match?(label.fetch('color'))
  abort "Missing label description: #{label['name']}" if label.fetch('description').strip.empty?
  abort "Label description too long: #{label['name']}" if label.fetch('description').length > 100
end
%w[P0 P1 P2 P3 status:\ needs-triage].each { |name| abort "Missing required label: #{name}" unless names.include?(name) }

config = YAML.safe_load(File.read(File.join(root, '.github/ISSUE_TEMPLATE/config.yml')))
abort 'Blank issues must be disabled' unless config.fetch('blank_issues_enabled') == false

Dir.glob(File.join(root, '.github/ISSUE_TEMPLATE/*.yml')).sort.each do |path|
  next if File.basename(path) == 'config.yml'
  form = YAML.safe_load(File.read(path))
  %w[name description body labels].each { |key| abort "#{path}: missing #{key}" unless form.key?(key) }
  abort "#{path}: unknown static label" unless (form.fetch('labels') - names).empty?
  abort "#{path}: new issues must start in triage" unless form.fetch('labels').include?('status: needs-triage')
  ids = form.fetch('body').map { |field| field['id'] }.compact
  abort "#{path}: duplicate field IDs" unless ids.uniq == ids
  fields = form.fetch('body').to_h { |field| [field['id'], field] }
  area = fields.fetch('affected-area') { abort "#{path}: missing affected-area" }
  unless area['type'] == 'dropdown' && area.fetch('attributes')['multiple'] == true &&
         area.fetch('validations')['required'] == true
    abort "#{path}: affected-area must be a required multi-select dropdown"
  end
  if File.basename(path) == 'feature-request.yml'
    diagnostic_ids = %w[app-version build-uuid distribution operating-system package-format logs]
    abort "#{path}: feature requests must not collect diagnostic fields" unless (ids & diagnostic_ids).empty?
    platform = fields.fetch('affected-platforms')
    abort "#{path}: affected-platforms must be optional" unless platform.fetch('validations')['required'] == false
  end
  if File.basename(path) == 'bug-report.yml'
    %w[app-version operating-system operating-system-version logs].each do |id|
      abort "#{path}: bug reports must collect #{id}" unless ids.include?(id)
    end
    logs = fields.fetch('logs').fetch('attributes')
    abort "#{path}: logs field must allow attachments (no render)" if logs.key?('render')
  end
  form.fetch('body').each do |field|
    next unless field['type'] == 'dropdown'
    attributes = field.fetch('attributes')
    options = attributes.fetch('options')
    abort "#{path}: empty dropdown" if options.empty?
    abort "#{path}: duplicate dropdown options" unless options.uniq == options
    if attributes['multiple'] == true
      # GitHub 用 ", " 拼接多选结果，选项内含逗号会被分流脚本拆成无效片段
      bad = options.find { |option| option.include?(',') }
      abort "#{path}: multi-select option must not contain commas: #{bad}" if bad
      next
    end
    unless options.first == '请选择 / Please select' && attributes['default'].eql?(0)
      abort "#{path}: single-select dropdown #{field['id']} must default to the neutral prompt"
    end
  end
end
puts 'Issue forms and label catalog are valid'
